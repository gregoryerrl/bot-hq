import { execGit } from "./git";
import { db, gitRemotes } from "./db";
import { and, eq } from "drizzle-orm";

export interface ParsedRemote {
  provider: "github" | "gitlab" | "bitbucket" | "custom";
  baseUrl: string;
  owner: string | null;
  repo: string | null;
}

export interface DetectedRemote {
  gitName: string;
  url: string;
  provider: "github" | "gitlab" | "bitbucket" | "custom";
  baseUrl: string;
  owner: string | null;
  repo: string | null;
}

const PROVIDER_MAP: Record<string, "github" | "gitlab" | "bitbucket"> = {
  "github.com": "github",
  "gitlab.com": "gitlab",
  "bitbucket.org": "bitbucket",
};

/**
 * Parse a git remote URL (HTTPS or SSH) into provider/owner/repo.
 */
export function parseRemoteUrl(url: string): ParsedRemote | null {
  try {
    // SSH format: git@github.com:owner/repo.git
    const sshMatch = url.match(/^[\w.-]+@([\w.-]+):(.+?)(?:\.git)?$/);
    if (sshMatch) {
      const hostname = sshMatch[1];
      const pathParts = sshMatch[2].split("/");
      const provider = PROVIDER_MAP[hostname] || "custom";
      const baseUrl = `https://${hostname}`;
      return {
        provider,
        baseUrl,
        owner: pathParts.length >= 2 ? pathParts[0] : null,
        repo: pathParts.length >= 2 ? pathParts[pathParts.length - 1] : null,
      };
    }

    // HTTPS format: https://github.com/owner/repo.git
    const parsed = new URL(url);
    const hostname = parsed.hostname;
    const provider = PROVIDER_MAP[hostname] || "custom";
    const baseUrl = `${parsed.protocol}//${hostname}`;
    const pathSegments = parsed.pathname.split("/").filter(Boolean);

    let owner: string | null = null;
    let repo: string | null = null;
    if (pathSegments.length >= 2) {
      owner = pathSegments[0];
      repo = pathSegments[1].replace(/\.git$/, "");
    }

    return { provider, baseUrl, owner, repo };
  } catch {
    return null;
  }
}

/**
 * Detect git remotes for a repository by running `git remote -v`.
 */
export async function detectGitRemotes(repoPath: string): Promise<DetectedRemote[]> {
  const { stdout } = await execGit(repoPath, ["remote", "-v"]);
  const lines = stdout.trim().split("\n").filter(Boolean);

  const seen = new Map<string, DetectedRemote>();

  for (const line of lines) {
    const match = line.match(/^(\S+)\t(\S+)\s+\((fetch|push)\)$/);
    if (!match) continue;

    const [, gitName, url, type] = match;

    // Deduplicate by remote name, prefer fetch URL
    if (type === "fetch" || !seen.has(gitName)) {
      const parsed = parseRemoteUrl(url);
      if (parsed) {
        seen.set(gitName, {
          gitName,
          url,
          provider: parsed.provider,
          baseUrl: parsed.baseUrl,
          owner: parsed.owner,
          repo: parsed.repo,
        });
      }
    }
  }

  return Array.from(seen.values());
}

/**
 * Create a workspace-scoped gitRemotes record from a detected remote.
 * Skips if a record with the same workspaceId + owner + repo already exists.
 */
export async function createWorkspaceRemote(
  workspaceId: number,
  remote: DetectedRemote
): Promise<boolean> {
  if (!remote.owner || !remote.repo) return false;

  // Duplicate check
  const existing = await db.query.gitRemotes.findFirst({
    where: and(
      eq(gitRemotes.workspaceId, workspaceId),
      eq(gitRemotes.owner, remote.owner),
      eq(gitRemotes.repo, remote.repo)
    ),
  });

  if (existing) return false;

  await db.insert(gitRemotes).values({
    workspaceId,
    provider: remote.provider,
    name: `${remote.owner}/${remote.repo}`,
    url: remote.baseUrl,
    owner: remote.owner,
    repo: remote.repo,
    credentials: null,
    createdAt: new Date(),
    updatedAt: new Date(),
  });

  return true;
}
