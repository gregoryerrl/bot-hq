import { NextRequest, NextResponse } from "next/server";
import { execGit, getWorkspaceRepoPath } from "@/lib/git";

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, remote, branch } = await request.json();

    if (!workspaceId) {
      return NextResponse.json(
        { error: "Missing workspaceId" },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    const args = ["pull"];
    if (remote) args.push(remote);
    if (branch) args.push(branch);

    const result = await execGit(repoPath, args);

    return NextResponse.json({
      success: true,
      output: result.stdout || result.stderr,
    });
  } catch (error) {
    console.error("Failed to pull:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to pull" },
      { status: 500 }
    );
  }
}
