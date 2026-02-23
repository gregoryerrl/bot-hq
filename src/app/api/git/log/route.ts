import { NextRequest, NextResponse } from "next/server";
import { execGit, getWorkspaceRepoPath } from "@/lib/git";

export async function GET(request: NextRequest) {
  const workspaceId = request.nextUrl.searchParams.get("workspaceId");
  const limit = request.nextUrl.searchParams.get("limit") || "50";
  const branch = request.nextUrl.searchParams.get("branch");

  if (!workspaceId) {
    return NextResponse.json(
      { error: "Missing workspaceId parameter" },
      { status: 400 }
    );
  }

  try {
    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    const args = [
      "log",
      `--format=%H\t%h\t%s\t%an\t%ar\t%D`,
      `-${limit}`,
    ];
    if (branch) args.push(branch);

    const { stdout } = await execGit(repoPath, args);

    const commits = stdout
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const [hash, shortHash, message, author, date, refs] = line.split("\t");
        return {
          hash,
          shortHash,
          message,
          author,
          date,
          refs: refs || "",
        };
      });

    return NextResponse.json({ commits });
  } catch (error) {
    console.error("Failed to get git log:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to get git log" },
      { status: 500 }
    );
  }
}
