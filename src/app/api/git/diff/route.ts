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

// Simple workspace-level mutex to prevent concurrent branch-switching
const branchLocks = new Map<string, Promise<void>>();

async function withBranchLock<T>(repoPath: string, fn: () => Promise<T>): Promise<T> {
  // Wait for any existing lock on this repo
  while (branchLocks.has(repoPath)) {
    await branchLocks.get(repoPath);
  }

  let resolve: () => void;
  const lockPromise = new Promise<void>((r) => { resolve = r; });
  branchLocks.set(repoPath, lockPromise);

  try {
    return await fn();
  } finally {
    branchLocks.delete(repoPath);
    resolve!();
  }
}

export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const repoPath = searchParams.get("path");
  const branch = searchParams.get("branch");
  const baseBranch = searchParams.get("base") || "main";
  const includeDiff = searchParams.get("includeDiff") === "true";
  const mode = searchParams.get("mode") || "branch";

  if (!repoPath || !branch) {
    return NextResponse.json(
      { error: "Missing path or branch parameter" },
      { status: 400 }
    );
  }

  try {
    const expandedPath = repoPath.replace("~", process.env.HOME || "");

    if (mode === "uncommitted") {
      // Show uncommitted changes on the branch
      // Requires switching to the branch, diffing, then switching back
      return await withBranchLock(expandedPath, async () => {
        // Remember current branch
        const { stdout: currentBranch } = await execAsync(
          `git rev-parse --abbrev-ref HEAD`,
          { cwd: expandedPath }
        );
        const originalBranch = currentBranch.trim();

        try {
          // Switch to the target branch
          await execAsync(`git checkout ${branch}`, { cwd: expandedPath });

          // Get uncommitted changes (staged + unstaged vs HEAD)
          const { stdout: numstat } = await execAsync(
            `git diff HEAD --numstat`,
            { cwd: expandedPath }
          );

          const { stdout: nameStatus } = await execAsync(
            `git diff HEAD --name-status`,
            { cwd: expandedPath }
          );

          // Parse results
          const statsLines = numstat.trim().split("\n").filter(Boolean);
          const statusLines = nameStatus.trim().split("\n").filter(Boolean);

          const statusMap: Record<string, string> = {};
          for (const line of statusLines) {
            const [status, ...pathParts] = line.split("\t");
            const filePath = pathParts[pathParts.length - 1];
            statusMap[filePath] = status;
          }

          const files: FileDiff[] = [];
          let totalAdditions = 0;
          let totalDeletions = 0;

          for (const line of statsLines) {
            const [addStr, delStr, filePath] = line.split("\t");
            const additions = addStr === "-" ? 0 : parseInt(addStr);
            const deletions = delStr === "-" ? 0 : parseInt(delStr);

            totalAdditions += additions;
            totalDeletions += deletions;

            const statusChar = statusMap[filePath] || "M";
            let status: FileDiff["status"] = "modified";
            if (statusChar === "A") status = "added";
            else if (statusChar === "D") status = "deleted";
            else if (statusChar.startsWith("R")) status = "renamed";

            const file: FileDiff = { path: filePath, additions, deletions, status };

            if (includeDiff) {
              try {
                const { stdout: fileDiff } = await execAsync(
                  `git diff HEAD -- "${filePath}"`,
                  { cwd: expandedPath }
                );
                file.diff = fileDiff;
              } catch {
                // File might be binary or deleted
              }
            }

            files.push(file);
          }

          const summary: DiffSummary = {
            files,
            totalAdditions,
            totalDeletions,
            commitMessages: [],
          };

          return NextResponse.json(summary);
        } finally {
          // Always switch back to the original branch
          await execAsync(`git checkout ${originalBranch}`, { cwd: expandedPath }).catch(() => {});
        }
      });
    }

    // Default mode=branch: compare branch to base using three-dot diff
    const { stdout: numstat } = await execAsync(
      `git diff ${baseBranch}...${branch} --numstat`,
      { cwd: expandedPath }
    );

    const { stdout: nameStatus } = await execAsync(
      `git diff ${baseBranch}...${branch} --name-status`,
      { cwd: expandedPath }
    );

    const { stdout: logOutput } = await execAsync(
      `git log ${baseBranch}..${branch} --format="%s" --reverse`,
      { cwd: expandedPath }
    );

    const statsLines = numstat.trim().split("\n").filter(Boolean);
    const statusLines = nameStatus.trim().split("\n").filter(Boolean);

    const statusMap: Record<string, string> = {};
    for (const line of statusLines) {
      const [status, ...pathParts] = line.split("\t");
      const filePath = pathParts[pathParts.length - 1];
      statusMap[filePath] = status;
    }

    const files: FileDiff[] = [];
    let totalAdditions = 0;
    let totalDeletions = 0;

    for (const line of statsLines) {
      const [addStr, delStr, filePath] = line.split("\t");
      const additions = addStr === "-" ? 0 : parseInt(addStr);
      const deletions = delStr === "-" ? 0 : parseInt(delStr);

      totalAdditions += additions;
      totalDeletions += deletions;

      const statusChar = statusMap[filePath] || "M";
      let status: FileDiff["status"] = "modified";
      if (statusChar === "A") status = "added";
      else if (statusChar === "D") status = "deleted";
      else if (statusChar.startsWith("R")) status = "renamed";

      const file: FileDiff = { path: filePath, additions, deletions, status };

      if (includeDiff) {
        try {
          const { stdout: fileDiff } = await execAsync(
            `git diff ${baseBranch}...${branch} -- "${filePath}"`,
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
