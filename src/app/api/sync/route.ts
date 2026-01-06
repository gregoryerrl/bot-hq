import { NextRequest, NextResponse } from "next/server";
import { syncAllWorkspaces, syncWorkspaceIssues } from "@/lib/sync";

export async function POST(request: NextRequest) {
  try {
    const body = await request.json().catch(() => ({}));
    const { workspaceId } = body;

    if (workspaceId) {
      const result = await syncWorkspaceIssues(workspaceId);
      return NextResponse.json(result);
    } else {
      const result = await syncAllWorkspaces();
      return NextResponse.json(result);
    }
  } catch (error) {
    console.error("Sync failed:", error);
    return NextResponse.json(
      { error: "Sync failed" },
      { status: 500 }
    );
  }
}
