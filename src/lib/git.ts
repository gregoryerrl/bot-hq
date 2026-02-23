import { execFile } from "child_process";
import { promisify } from "util";
import { db, workspaces } from "./db/index";
import { eq } from "drizzle-orm";

const execFileAsync = promisify(execFile);

export interface GitResult {
  stdout: string;
  stderr: string;
}

/**
 * Execute a git command at a given repo path.
 * Uses execFile (no shell) to avoid shell interpretation of special characters
 * like % in git format strings.
 */
export async function execGit(
  repoPath: string,
  args: string[]
): Promise<GitResult> {
  const expandedPath = repoPath.replace("~", process.env.HOME || "");
  const { stdout, stderr } = await execFileAsync("git", args, {
    cwd: expandedPath,
    maxBuffer: 10 * 1024 * 1024, // 10MB for large diffs/logs
  });
  return { stdout, stderr };
}

/**
 * Look up the repo path for a workspace by its ID.
 */
export async function getWorkspaceRepoPath(
  workspaceId: number
): Promise<string> {
  const workspace = await db.query.workspaces.findFirst({
    where: eq(workspaces.id, workspaceId),
  });

  if (!workspace) {
    throw new Error(`Workspace ${workspaceId} not found`);
  }

  return workspace.repoPath.replace("~", process.env.HOME || "");
}

/**
 * Workspace-level mutex to prevent concurrent branch-switching.
 * Shared across routes that modify branch state.
 */
const branchLocks = new Map<string, Promise<void>>();

export async function withBranchLock<T>(
  repoPath: string,
  fn: () => Promise<T>
): Promise<T> {
  while (branchLocks.has(repoPath)) {
    await branchLocks.get(repoPath);
  }

  let resolve: () => void;
  const lockPromise = new Promise<void>((r) => {
    resolve = r;
  });
  branchLocks.set(repoPath, lockPromise);

  try {
    return await fn();
  } finally {
    branchLocks.delete(repoPath);
    resolve!();
  }
}
