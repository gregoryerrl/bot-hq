import { NextRequest, NextResponse } from "next/server";
import { exec } from "child_process";
import { promisify } from "util";

const execAsync = promisify(exec);

interface FileDiff {
  path: string;
  additions: number;
  deletions: number;
  status: "added" | "modified" | "deleted" | "renamed";
  diff?: string;
}

interface DiffSummary {
  files: FileDiff[];
  totalAdditions: number;
  totalDeletions: number;
  commitMessages: string[];
}

export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const repoPath = searchParams.get("path");
  const branch = searchParams.get("branch");
  const baseBranch = searchParams.get("base") || "main";
  const includeDiff = searchParams.get("includeDiff") === "true";

  if (!repoPath || !branch) {
    return NextResponse.json(
      { error: "Missing path or branch parameter" },
      { status: 400 }
    );
  }

  try {
    const expandedPath = repoPath.replace("~", process.env.HOME || "");

    // Get file stats (additions/deletions per file)
    const { stdout: numstat } = await execAsync(
      `git diff ${baseBranch}...${branch} --numstat`,
      { cwd: expandedPath }
    );

    // Get file status (added/modified/deleted)
    const { stdout: nameStatus } = await execAsync(
      `git diff ${baseBranch}...${branch} --name-status`,
      { cwd: expandedPath }
    );

    // Get commit messages on this branch
    const { stdout: logOutput } = await execAsync(
      `git log ${baseBranch}..${branch} --format="%s" --reverse`,
      { cwd: expandedPath }
    );

    // Parse file stats
    const statsLines = numstat.trim().split("\n").filter(Boolean);
    const statusLines = nameStatus.trim().split("\n").filter(Boolean);

    const statusMap: Record<string, string> = {};
    for (const line of statusLines) {
      const [status, ...pathParts] = line.split("\t");
      const path = pathParts[pathParts.length - 1]; // Handle renames
      statusMap[path] = status;
    }

    const files: FileDiff[] = [];
    let totalAdditions = 0;
    let totalDeletions = 0;

    for (const line of statsLines) {
      const [addStr, delStr, path] = line.split("\t");
      const additions = addStr === "-" ? 0 : parseInt(addStr);
      const deletions = delStr === "-" ? 0 : parseInt(delStr);

      totalAdditions += additions;
      totalDeletions += deletions;

      const statusChar = statusMap[path] || "M";
      let status: FileDiff["status"] = "modified";
      if (statusChar === "A") status = "added";
      else if (statusChar === "D") status = "deleted";
      else if (statusChar.startsWith("R")) status = "renamed";

      const file: FileDiff = {
        path,
        additions,
        deletions,
        status,
      };

      // Include actual diff content if requested
      if (includeDiff) {
        try {
          const { stdout: fileDiff } = await execAsync(
            `git diff ${baseBranch}...${branch} -- "${path}"`,
            { cwd: expandedPath }
          );
          file.diff = fileDiff;
        } catch {
          // File might be binary or deleted
        }
      }

      files.push(file);
    }

    const commitMessages = logOutput.trim().split("\n").filter(Boolean);

    const summary: DiffSummary = {
      files,
      totalAdditions,
      totalDeletions,
      commitMessages,
    };

    return NextResponse.json(summary);
  } catch (error) {
    console.error("Failed to get diff:", error);
    return NextResponse.json(
      { error: "Failed to get diff" },
      { status: 500 }
    );
  }
}
