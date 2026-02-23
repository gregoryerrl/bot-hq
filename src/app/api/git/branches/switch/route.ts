import { NextRequest, NextResponse } from "next/server";
import { execGit, getWorkspaceRepoPath, withBranchLock } from "@/lib/git";

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, branch } = await request.json();

    if (!workspaceId || !branch) {
      return NextResponse.json(
        { error: "Missing workspaceId or branch" },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    await withBranchLock(repoPath, async () => {
      await execGit(repoPath, ["checkout", branch]);
    });

    return NextResponse.json({ success: true, branch });
  } catch (error) {
    console.error("Failed to switch branch:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to switch branch" },
      { status: 500 }
    );
  }
}
