import { NextRequest, NextResponse } from "next/server";
import { execGit, getWorkspaceRepoPath } from "@/lib/git";

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, remote, branch, setUpstream } = await request.json();

    if (!workspaceId) {
      return NextResponse.json(
        { error: "Missing workspaceId" },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    const args = ["push"];
    if (setUpstream) args.push("--set-upstream");
    if (remote) args.push(remote);
    if (branch) args.push(branch);

    const result = await execGit(repoPath, args);

    return NextResponse.json({
      success: true,
      output: result.stderr || result.stdout,
    });
  } catch (error) {
    console.error("Failed to push:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to push" },
      { status: 500 }
    );
  }
}
