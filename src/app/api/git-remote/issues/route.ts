import { NextResponse } from "next/server";
import { db, gitRemotes, workspaces, tasks } from "@/lib/db";
import { eq, and } from "drizzle-orm";

function decryptCredentials(encrypted: string): { token: string } | null {
  try {
    return JSON.parse(Buffer.from(encrypted, "base64").toString("utf-8"));
  } catch {
    return null;
  }
}

interface GitHubIssue {
  number: number;
  title: string;
  body: string | null;
  state: string;
  labels: { name: string }[];
  html_url: string;
}

export async function GET() {
  try {
    // Get all workspace-scoped remotes with owner/repo configured
    const remotes = await db
      .select()
      .from(gitRemotes)
      .where(eq(gitRemotes.provider, "github"));

    const workspaceIssues: {
      workspaceId: number;
      workspaceName: string;
      owner: string;
      repo: string;
      issues: {
        number: number;
        title: string;
        body: string | null;
        state: string;
        labels: string[];
        url: string;
        hasTask: boolean;
        taskId?: number;
      }[];
    }[] = [];

    // Find global remote with credentials
    const globalRemote = remotes.find(r => !r.workspaceId && r.credentials);
    let token: string | null = null;

    if (globalRemote?.credentials) {
      const creds = decryptCredentials(globalRemote.credentials);
      token = creds?.token || null;
    }

    // Process workspace remotes
    for (const remote of remotes) {
      if (!remote.workspaceId || !remote.owner || !remote.repo) continue;

      // Get workspace name
      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, remote.workspaceId),
      });

      if (!workspace) continue;

      // Use remote's own credentials if available, otherwise use global
      let remoteToken = token;
      if (remote.credentials) {
        const creds = decryptCredentials(remote.credentials);
        if (creds?.token) remoteToken = creds.token;
      }

      if (!remoteToken) continue;

      try {
        // Fetch issues from GitHub
        const response = await fetch(
          `https://api.github.com/repos/${remote.owner}/${remote.repo}/issues?state=open&per_page=100`,
          {
            headers: {
              Authorization: `token ${remoteToken}`,
              Accept: "application/vnd.github.v3+json",
              "User-Agent": "bot-hq",
            },
          }
        );

        if (!response.ok) {
          console.error(`Failed to fetch issues for ${remote.owner}/${remote.repo}: ${response.status}`);
          continue;
        }

        const issues: GitHubIssue[] = await response.json();

        // Filter out pull requests (GitHub API returns them as issues too)
        const realIssues = issues.filter(i => !("pull_request" in i));

        // Check which issues have tasks
        const issuesWithStatus = await Promise.all(
          realIssues.map(async (issue) => {
            const existingTask = await db.query.tasks.findFirst({
              where: and(
                eq(tasks.workspaceId, remote.workspaceId!),
                eq(tasks.sourceRemoteId, remote.id),
                eq(tasks.sourceRef, String(issue.number))
              ),
            });

            return {
              number: issue.number,
              title: issue.title,
              body: issue.body,
              state: issue.state,
              labels: issue.labels.map(l => l.name),
              url: issue.html_url,
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
      }
    }

    const totalIssues = workspaceIssues.reduce((sum, w) => sum + w.issues.length, 0);
    const issuesWithTasks = workspaceIssues.reduce(
      (sum, w) => sum + w.issues.filter(i => i.hasTask).length,
      0
    );

    return NextResponse.json({
      workspaces: workspaceIssues,
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
