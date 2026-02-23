import { NextRequest, NextResponse } from "next/server";
import { execGit, getWorkspaceRepoPath } from "@/lib/git";

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, message } = await request.json();

    if (!workspaceId || !message) {
      return NextResponse.json(
        { error: "Missing workspaceId or message" },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    await execGit(repoPath, ["commit", "-m", message]);

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to commit:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to commit" },
      { status: 500 }
    );
  }
}
