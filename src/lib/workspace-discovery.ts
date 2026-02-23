import fs from "fs";
import path from "path";
import { getScopePath } from "@/lib/settings";
import { db, workspaces } from "@/lib/db";
import { detectGitRemotes } from "@/lib/git-remote-utils";

export interface WorkspaceSuggestion {
  name: string;
  repoPath: string;
  remotes?: { gitName: string; provider: string; owner: string | null; repo: string | null }[];
}

export interface CleanupSuggestion {
  name: string;
  path: string;
  reason: string;
  lastModified: string;
  isEmpty: boolean;
  hasGit: boolean;
}

const HIDDEN_DIR_PATTERN = /^\./;
const STALE_MONTHS = 6;

/**
 * Find git repos in scope directory that aren't tracked as workspaces.
 */
export async function discoverWorkspaces(): Promise<WorkspaceSuggestion[]> {
  const scopePath = await getScopePath();

  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(scopePath, { withFileTypes: true });
  } catch {
    return [];
  }

  // Get all known workspace repo paths
  const allWorkspaces = await db.select({ repoPath: workspaces.repoPath }).from(workspaces);
  const knownPaths = new Set(allWorkspaces.map((w) => w.repoPath));

  const suggestions: WorkspaceSuggestion[] = [];

  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    if (HIDDEN_DIR_PATTERN.test(entry.name)) continue;

    const dirPath = path.join(scopePath, entry.name);
    const gitPath = path.join(dirPath, ".git");

    // Check if it's a git repo and not already tracked
    if (fs.existsSync(gitPath) && !knownPaths.has(dirPath)) {
      let remotes: WorkspaceSuggestion["remotes"];
      try {
        const detected = await detectGitRemotes(dirPath);
        remotes = detected.map((r) => ({
          gitName: r.gitName,
          provider: r.provider,
          owner: r.owner,
          repo: r.repo,
        }));
      } catch {
        // Remote detection failure shouldn't prevent the suggestion
      }
      suggestions.push({ name: entry.name, repoPath: dirPath, remotes });
    }
  }

  return suggestions;
}

/**
 * Find folders in scope directory that might need cleanup.
 */
export async function scanForCleanup(): Promise<CleanupSuggestion[]> {
  const scopePath = await getScopePath();

  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(scopePath, { withFileTypes: true });
  } catch {
    return [];
  }

  // Get all known workspace repo paths
  const allWorkspaces = await db.select({ repoPath: workspaces.repoPath }).from(workspaces);
  const knownPaths = new Set(allWorkspaces.map((w) => w.repoPath));

  const suggestions: CleanupSuggestion[] = [];
  const now = new Date();
  const staleThreshold = new Date(now);
  staleThreshold.setMonth(staleThreshold.getMonth() - STALE_MONTHS);

  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    if (HIDDEN_DIR_PATTERN.test(entry.name)) continue;

    const dirPath = path.join(scopePath, entry.name);

    // Skip if it's already a tracked workspace
    if (knownPaths.has(dirPath)) continue;

    const hasGit = fs.existsSync(path.join(dirPath, ".git"));

    let isEmpty = false;
    try {
      isEmpty = fs.readdirSync(dirPath).length === 0;
    } catch {
      continue;
    }

    let mtime: Date;
    try {
      mtime = fs.statSync(dirPath).mtime;
    } catch {
      continue;
    }

    const isStale = mtime < staleThreshold;

    // Determine reason
    let reason: string | null = null;
    if (isEmpty) {
      reason = "Empty directory";
    } else if (!hasGit) {
      reason = "No git repository";
    } else if (isStale) {
      const monthsAgo = Math.floor(
        (now.getTime() - mtime.getTime()) / (1000 * 60 * 60 * 24 * 30)
      );
      reason = `Not modified in ${monthsAgo} months`;
    }

    if (reason) {
      suggestions.push({
        name: entry.name,
        path: dirPath,
        reason,
        lastModified: mtime.toISOString(),
        isEmpty,
        hasGit,
      });
    }
  }

  return suggestions;
}
