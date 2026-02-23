import { NextResponse } from "next/server";
import { db, gitRemotes, workspaces, tasks } from "@/lib/db";
import { eq, and } from "drizzle-orm";
import { execFile } from "child_process";
import { promisify } from "util";

const execFileAsync = promisify(execFile);

function decryptCredentials(encrypted: string): { token: string } | null {
  try {
    return JSON.parse(Buffer.from(encrypted, "base64").toString("utf-8"));
  } catch {
    return null;
  }
}

interface NormalizedIssue {
  number: number;
  title: string;
  body: string | null;
  state: string;
  labels: string[];
  url: string;
  author: string;
  createdAt: string;
  commentsCount: number;
}

/**
 * Fetch issues via `gh` CLI (uses machine's GitHub auth).
 */
async function fetchIssuesViaGh(owner: string, repo: string): Promise<NormalizedIssue[]> {
  const { stdout } = await execFileAsync("gh", [
    "issue", "list",
    "--repo", `${owner}/${repo}`,
    "--state", "open",
    "--limit", "100",
    "--json", "number,title,body,state,labels,url,author,createdAt,comments",
  ], { timeout: 30000 });

  const issues = JSON.parse(stdout);
  return issues.map((i: {
    number: number;
    title: string;
    body: string;
    state: string;
    labels: { name: string }[];
    url: string;
    author: { login: string };
    createdAt: string;
    comments: unknown[];
  }) => ({
    number: i.number,
    title: i.title,
    body: i.body || null,
    state: i.state,
    labels: i.labels.map((l) => l.name),
    url: i.url,
    author: i.author?.login || "unknown",
    createdAt: i.createdAt,
    commentsCount: i.comments?.length || 0,
  }));
}

/**
 * Fetch issues via GitHub API (uses stored token).
 */
async function fetchIssuesViaApi(
  owner: string,
  repo: string,
  token: string
): Promise<NormalizedIssue[]> {
  const response = await fetch(
    `https://api.github.com/repos/${owner}/${repo}/issues?state=open&per_page=100`,
    {
      headers: {
        Authorization: `token ${token}`,
        Accept: "application/vnd.github.v3+json",
        "User-Agent": "bot-hq",
      },
    }
  );

  if (!response.ok) {
    throw new Error(`GitHub API ${response.status}`);
  }

  const issues = await response.json();
  // Filter out pull requests (GitHub API returns them as issues too)
  return issues
    .filter((i: { pull_request?: unknown }) => !i.pull_request)
    .map((i: {
      number: number;
      title: string;
      body: string | null;
      state: string;
      labels: { name: string }[];
      html_url: string;
      user: { login: string };
      created_at: string;
      comments: number;
    }) => ({
      number: i.number,
      title: i.title,
      body: i.body,
      state: i.state,
      labels: i.labels.map((l) => l.name),
      url: i.html_url,
      author: i.user?.login || "unknown",
      createdAt: i.created_at,
      commentsCount: i.comments || 0,
    }));
}

export async function GET() {
  try {
    const remotes = await db
      .select()
      .from(gitRemotes)
      .where(eq(gitRemotes.provider, "github"));

    const globalRemote = remotes.find((r) => !r.workspaceId && r.credentials);
    let globalToken: string | null = null;
    if (globalRemote?.credentials) {
      const creds = decryptCredentials(globalRemote.credentials);
      globalToken = creds?.token || null;
    }

    let ghAvailable = false;
    try {
      await execFileAsync("gh", ["auth", "status"], { timeout: 5000 });
      ghAvailable = true;
    } catch {
      // gh CLI not available or not authenticated
    }

    const workspaceIssues: {
      workspaceId: number;
      workspaceName: string;
      owner: string;
      repo: string;
      issues: (NormalizedIssue & { hasTask: boolean; taskId?: number })[];
    }[] = [];

    const skippedWorkspaces: {
      workspaceId: number;
      workspaceName: string;
      owner: string;
      repo: string;
      reason: string;
    }[] = [];

    for (const remote of remotes) {
      if (!remote.workspaceId || !remote.owner || !remote.repo) continue;

      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, remote.workspaceId),
      });
      if (!workspace) continue;

      let remoteToken = globalToken;
      if (remote.credentials) {
        const creds = decryptCredentials(remote.credentials);
        if (creds?.token) remoteToken = creds.token;
      }

      try {
        let rawIssues: NormalizedIssue[];

        if (remoteToken) {
          rawIssues = await fetchIssuesViaApi(remote.owner, remote.repo, remoteToken);
        } else if (ghAvailable) {
          rawIssues = await fetchIssuesViaGh(remote.owner, remote.repo);
        } else {
          skippedWorkspaces.push({
            workspaceId: remote.workspaceId,
            workspaceName: workspace.name,
            owner: remote.owner,
            repo: remote.repo,
            reason: "no_token",
          });
          continue;
        }

        const issuesWithStatus = await Promise.all(
          rawIssues.map(async (issue) => {
            const existingTask = await db.query.tasks.findFirst({
              where: and(
                eq(tasks.workspaceId, remote.workspaceId!),
                eq(tasks.sourceRemoteId, remote.id),
                eq(tasks.sourceRef, String(issue.number))
              ),
            });

            return {
              ...issue,
              hasTask: !!existingTask,
              taskId: existingTask?.id,
            };
          })
        );

        workspaceIssues.push({
          workspaceId: remote.workspaceId,
          workspaceName: workspace.name,
          owner: remote.owner,
          repo: remote.repo,
          issues: issuesWithStatus,
        });
      } catch (error) {
        console.error(`Error fetching issues for ${remote.owner}/${remote.repo}:`, error);
        skippedWorkspaces.push({
          workspaceId: remote.workspaceId,
          workspaceName: workspace.name,
          owner: remote.owner,
          repo: remote.repo,
          reason: "fetch_error",
        });
      }
    }

    const totalIssues = workspaceIssues.reduce((sum, w) => sum + w.issues.length, 0);
    const issuesWithTasks = workspaceIssues.reduce(
      (sum, w) => sum + w.issues.filter((i) => i.hasTask).length,
      0
    );

    return NextResponse.json({
      workspaces: workspaceIssues,
      skippedWorkspaces,
      hasGlobalToken: !!globalToken,
      ghAvailable,
      totalIssues,
      issuesWithTasks,
    });
  } catch (error) {
    console.error("Failed to fetch issues:", error);
    return NextResponse.json(
      { error: "Failed to fetch issues" },
      { status: 500 }
    );
  }
}
