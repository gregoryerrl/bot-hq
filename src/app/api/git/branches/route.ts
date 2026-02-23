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

    // Get current branch
    const { stdout: currentBranch } = await execGit(repoPath, [
      "rev-parse",
      "--abbrev-ref",
      "HEAD",
    ]);

    // Get all branches with last commit info
    const { stdout: branchOutput } = await execGit(repoPath, [
      "branch",
      "-a",
      "--format=%(refname:short)\t%(objectname:short)\t%(committerdate:relative)\t%(subject)",
    ]);

    const branches = branchOutput
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const [name, hash, date, ...msgParts] = line.split("\t");
        return {
          name,
          hash,
          date,
          message: msgParts.join("\t"),
          current: name === currentBranch.trim(),
          isRemote: name.startsWith("origin/"),
        };
      });

    const local = branches.filter((b) => !b.isRemote);
    const remote = branches.filter((b) => b.isRemote);

    return NextResponse.json({
      current: currentBranch.trim(),
      local,
      remote,
    });
  } catch (error) {
    console.error("Failed to list branches:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to list branches" },
      { status: 500 }
    );
  }
}

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, name, from } = await request.json();

    if (!workspaceId || !name) {
      return NextResponse.json(
        { error: "Missing workspaceId or name" },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));
    const args = ["checkout", "-b", name];
    if (from) args.push(from);

    await execGit(repoPath, args);

    return NextResponse.json({ success: true, branch: name });
  } catch (error) {
    console.error("Failed to create branch:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to create branch" },
      { status: 500 }
    );
  }
}

export async function DELETE(request: NextRequest) {
  try {
    const { workspaceId, name } = await request.json();

    if (!workspaceId || !name) {
      return NextResponse.json(
        { error: "Missing workspaceId or name" },
        { status: 400 }
      );
    }

    const repoPath = await getWorkspaceRepoPath(Number(workspaceId));

    // Prevent deleting current branch or main
    const { stdout: currentBranch } = await execGit(repoPath, [
      "rev-parse",
      "--abbrev-ref",
      "HEAD",
    ]);

    if (name === currentBranch.trim()) {
      return NextResponse.json(
        { error: "Cannot delete the current branch" },
        { status: 400 }
      );
    }

    if (name === "main" || name === "master") {
      return NextResponse.json(
        { error: "Cannot delete main/master branch" },
        { status: 400 }
      );
    }

    await execGit(repoPath, ["branch", "-d", name]);

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to delete branch:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to delete branch" },
      { status: 500 }
    );
  }
}
