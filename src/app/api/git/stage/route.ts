import { NextRequest, NextResponse } from "next/server";
import { execGit, getWorkspaceRepoPath } from "@/lib/git";

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, files, action } = await request.json();

    if (!workspaceId || !files || !action) {
      return NextResponse.json(
        { error: "Missing workspaceId, files, or action" },
        { status: 400 }
      );
    }

    if (action !== "stage" && action !== "unstage") {
      return NextResponse.json(
        { error: 'action must be "stage" or "unstage"' },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    if (action === "stage") {
      await execGit(repoPath, ["add", "--", ...files]);
    } else {
      await execGit(repoPath, ["restore", "--staged", "--", ...files]);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to stage/unstage files:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to stage/unstage files" },
      { status: 500 }
    );
  }
}
