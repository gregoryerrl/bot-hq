import { NextRequest, NextResponse } from "next/server";
import { execGit, getWorkspaceRepoPath } from "@/lib/git";

export async function GET(request: NextRequest) {
  const workspaceId = request.nextUrl.searchParams.get("workspaceId");

  if (!workspaceId) {
    return NextResponse.json(
      { error: "Missing workspaceId parameter" },
      { status: 400 }
    );
  }

  try {
    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    // Get branch info + porcelain status
    const [statusResult, stashResult] = await Promise.all([
      execGit(repoPath, ["status", "--porcelain=v2", "--branch"]),
      execGit(repoPath, ["stash", "list", "--format=%gd"]),
    ]);

    const lines = statusResult.stdout.split("\n");
    let branch = "";
    let upstream = "";
    let ahead = 0;
    let behind = 0;
    const staged: string[] = [];
    const modified: string[] = [];
    const untracked: string[] = [];

    for (const line of lines) {
      if (line.startsWith("# branch.head ")) {
        branch = line.replace("# branch.head ", "");
      } else if (line.startsWith("# branch.upstream ")) {
        upstream = line.replace("# branch.upstream ", "");
      } else if (line.startsWith("# branch.ab ")) {
        const match = line.match(/\+(\d+) -(\d+)/);
        if (match) {
          ahead = parseInt(match[1]);
          behind = parseInt(match[2]);
        }
      } else if (line.startsWith("1 ") || line.startsWith("2 ")) {
        // Changed entry: "1 XY sub mH mI mW hH hO path" or renamed "2 XY ... path\torigPath"
        const parts = line.split(" ");
        const xy = parts[1];
        const path = parts.slice(8).join(" ").split("\t")[0];

        if (xy[0] !== ".") staged.push(path);
        if (xy[1] !== ".") modified.push(path);
      } else if (line.startsWith("? ")) {
        untracked.push(line.substring(2));
      }
    }

    const stashCount = stashResult.stdout.trim()
      ? stashResult.stdout.trim().split("\n").length
      : 0;

    return NextResponse.json({
      branch,
      upstream,
      ahead,
      behind,
      staged,
      modified,
      untracked,
      clean: staged.length === 0 && modified.length === 0 && untracked.length === 0,
      stashCount,
    });
  } catch (error) {
    console.error("Failed to get git status:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to get git status" },
      { status: 500 }
    );
  }
}
