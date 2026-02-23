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

    const { stdout } = await execGit(repoPath, [
      "stash",
      "list",
      "--format=%gd\t%gs\t%ci",
    ]);

    const stashes = stdout
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const [index, message, date] = line.split("\t");
        return { index, message, date };
      });

    return NextResponse.json({ stashes });
  } catch (error) {
    console.error("Failed to list stashes:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to list stashes" },
      { status: 500 }
    );
  }
}

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, action, index, message } = await request.json();

    if (!workspaceId || !action) {
      return NextResponse.json(
        { error: "Missing workspaceId or action" },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    let args: string[];
    switch (action) {
      case "save":
        args = ["stash", "push"];
        if (message) args.push("-m", message);
        break;
      case "pop":
        args = ["stash", "pop"];
        if (index !== undefined) args.push(`stash@{${index}}`);
        break;
      case "apply":
        args = ["stash", "apply"];
        if (index !== undefined) args.push(`stash@{${index}}`);
        break;
      case "drop":
        args = ["stash", "drop"];
        if (index !== undefined) args.push(`stash@{${index}}`);
        break;
      default:
        return NextResponse.json(
          { error: 'action must be "save", "pop", "apply", or "drop"' },
          { status: 400 }
        );
    }

    const result = await execGit(repoPath, args);

    return NextResponse.json({
      success: true,
      output: result.stdout || result.stderr,
    });
  } catch (error) {
    console.error("Failed to perform stash action:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to perform stash action" },
      { status: 500 }
    );
  }
}
