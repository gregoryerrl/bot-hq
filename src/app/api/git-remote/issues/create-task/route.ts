import { NextRequest, NextResponse } from "next/server";
import { db, gitRemotes, tasks, logs } from "@/lib/db";
import { eq, and, isNull } from "drizzle-orm";

function decryptCredentials(encrypted: string): { token: string } | null {
  try {
    return JSON.parse(Buffer.from(encrypted, "base64").toString("utf-8"));
  } catch {
    return null;
  }
}

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, issueNumber } = await request.json();

    if (!workspaceId || !issueNumber) {
      return NextResponse.json(
        { error: "Missing required fields: workspaceId, issueNumber" },
        { status: 400 }
      );
    }

    // Get the git remote for this workspace
    const remote = await db.query.gitRemotes.findFirst({
      where: and(
        eq(gitRemotes.workspaceId, workspaceId),
        eq(gitRemotes.provider, "github")
      ),
    });

    if (!remote) {
      return NextResponse.json(
        { error: "No GitHub remote configured for this workspace" },
        { status: 400 }
      );
    }

    if (!remote.owner || !remote.repo) {
      return NextResponse.json(
        { error: "GitHub remote owner/repo not configured" },
        { status: 400 }
      );
    }

    // Check if task already exists
    const existingTask = await db.query.tasks.findFirst({
      where: and(
        eq(tasks.workspaceId, workspaceId),
        eq(tasks.sourceRemoteId, remote.id),
        eq(tasks.sourceRef, String(issueNumber))
      ),
    });

    if (existingTask) {
      return NextResponse.json(
        { error: `Task already exists for issue #${issueNumber}`, taskId: existingTask.id },
        { status: 400 }
      );
    }

    // Get token for API call
    let token: string | null = null;

    if (remote.credentials) {
      const creds = decryptCredentials(remote.credentials);
      token = creds?.token || null;
    }

    if (!token) {
      // Try to get token from global remote
      const globalRemote = await db.query.gitRemotes.findFirst({
        where: and(
          eq(gitRemotes.provider, "github"),
          isNull(gitRemotes.workspaceId)
        ),
      });

      if (globalRemote?.credentials) {
        const creds = decryptCredentials(globalRemote.credentials);
        token = creds?.token || null;
      }
    }

    if (!token) {
      return NextResponse.json(
        { error: "No GitHub token available" },
        { status: 400 }
      );
    }

    // Fetch issue details from GitHub
    const response = await fetch(
      `https://api.github.com/repos/${remote.owner}/${remote.repo}/issues/${issueNumber}`,
      {
        headers: {
          Authorization: `token ${token}`,
          Accept: "application/vnd.github.v3+json",
          "User-Agent": "bot-hq",
        },
      }
    );

    if (!response.ok) {
      return NextResponse.json(
        { error: `Failed to fetch issue #${issueNumber} from GitHub` },
        { status: 400 }
      );
    }

    const issue = await response.json();

    // Create the task
    const [newTask] = await db
      .insert(tasks)
      .values({
        workspaceId,
        sourceRemoteId: remote.id,
        sourceRef: String(issue.number),
        title: issue.title,
        description: `${issue.body || ""}\n\n---\nGitHub Issue: ${issue.html_url}`,
        priority: 0,
        state: "new",
        updatedAt: new Date(),
      })
      .returning();

    await db.insert(logs).values({
      workspaceId,
      taskId: newTask.id,
      type: "agent",
      message: `Task created from GitHub issue #${issue.number}: ${issue.title}`,
    });

    return NextResponse.json({
      taskId: newTask.id,
      title: issue.title,
      message: `Task created from GitHub issue #${issue.number}`,
    });
  } catch (error) {
    console.error("Failed to create task from issue:", error);
    return NextResponse.json(
      { error: "Failed to create task" },
      { status: 500 }
    );
  }
}
