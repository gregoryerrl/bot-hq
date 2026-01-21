import { NextRequest, NextResponse } from "next/server";
import { db, tasks, pluginWorkspaceData, plugins, logs } from "@/lib/db";
import { getMcpManager } from "@/lib/plugins";
import { eq, and } from "drizzle-orm";

interface GitHubIssue {
  number: number;
  title: string;
  body: string | null;
  state: string;
  labels: string[];
  assignees: string[];
  url: string;
}

export async function POST(request: NextRequest) {
  try {
    const { workspaceId, issueNumber, priority } = await request.json() as {
      workspaceId: number;
      issueNumber: number;
      priority?: number;
    };

    if (!workspaceId || !issueNumber) {
      return NextResponse.json(
        { error: "workspaceId and issueNumber are required" },
        { status: 400 }
      );
    }

    // Get GitHub plugin
    const plugin = await db.query.plugins.findFirst({
      where: eq(plugins.name, "github"),
    });

    if (!plugin || !plugin.enabled) {
      return NextResponse.json(
        { error: "GitHub plugin not installed or disabled" },
        { status: 400 }
      );
    }

    // Check if task already exists
    const existingTask = await db.query.tasks.findFirst({
      where: and(
        eq(tasks.workspaceId, workspaceId),
        eq(tasks.sourcePluginId, plugin.id),
        eq(tasks.sourceRef, String(issueNumber))
      ),
    });

    if (existingTask) {
      return NextResponse.json(
        { error: "Task already exists for this issue", taskId: existingTask.id },
        { status: 409 }
      );
    }

    // Get workspace GitHub config
    const workspaceConfig = await db.query.pluginWorkspaceData.findFirst({
      where: and(
        eq(pluginWorkspaceData.pluginId, plugin.id),
        eq(pluginWorkspaceData.workspaceId, workspaceId)
      ),
    });

    if (!workspaceConfig) {
      return NextResponse.json(
        { error: "GitHub not configured for this workspace" },
        { status: 400 }
      );
    }

    const config = JSON.parse(workspaceConfig.data) as { owner?: string; repo?: string };

    if (!config.owner || !config.repo) {
      return NextResponse.json(
        { error: "GitHub owner/repo not configured" },
        { status: 400 }
      );
    }

    // Fetch issue details from GitHub
    const manager = getMcpManager();
    const issue = await manager.callTool("github", "github_get_issue", {
      owner: config.owner,
      repo: config.repo,
      issueNumber,
    }) as GitHubIssue;

    // Create the task
    const [newTask] = await db
      .insert(tasks)
      .values({
        workspaceId,
        sourcePluginId: plugin.id,
        sourceRef: String(issue.number),
        title: issue.title,
        description: `${issue.body || ""}\n\n---\nGitHub Issue: ${issue.url}`,
        priority: priority || 0,
        state: "new",
      })
      .returning();

    // Log the creation
    await db.insert(logs).values({
      workspaceId,
      taskId: newTask.id,
      type: "agent",
      message: `Task created from GitHub issue #${issue.number}: ${issue.title}`,
    });

    return NextResponse.json({
      success: true,
      taskId: newTask.id,
      title: issue.title,
      message: `Task created from GitHub issue #${issue.number}`,
    });
  } catch (error) {
    console.error("Failed to create task from issue:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to create task" },
      { status: 500 }
    );
  }
}
