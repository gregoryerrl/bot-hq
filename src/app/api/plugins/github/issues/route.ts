import { NextResponse } from "next/server";
import { db, workspaces, pluginWorkspaceData, plugins, tasks } from "@/lib/db";
import { getMcpManager, initializePlugins } from "@/lib/plugins";
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

interface WorkspaceIssues {
  workspaceId: number;
  workspaceName: string;
  owner: string;
  repo: string;
  issues: (GitHubIssue & { hasTask: boolean; taskId?: number })[];
}

export async function GET() {
  try {
    // Ensure plugins are initialized
    await initializePlugins();

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

    // Get all workspaces with GitHub configured
    const workspaceConfigs = await db
      .select({
        workspaceId: pluginWorkspaceData.workspaceId,
        data: pluginWorkspaceData.data,
      })
      .from(pluginWorkspaceData)
      .where(eq(pluginWorkspaceData.pluginId, plugin.id));

    const manager = getMcpManager();
    const allIssues: WorkspaceIssues[] = [];

    for (const config of workspaceConfigs) {
      try {
        const parsed = JSON.parse(config.data) as { owner?: string; repo?: string };
        if (!parsed.owner || !parsed.repo) continue;

        // Get workspace name
        const workspace = await db.query.workspaces.findFirst({
          where: eq(workspaces.id, config.workspaceId),
        });

        if (!workspace) continue;

        let issuesWithTaskStatus: (GitHubIssue & { hasTask: boolean; taskId?: number })[] = [];

        // Try to fetch issues from GitHub via MCP
        try {
          const result = await manager.callTool("github", "github_sync_issues", {
            owner: parsed.owner,
            repo: parsed.repo,
          }) as { issues: GitHubIssue[] };

          // Check which issues already have tasks
          issuesWithTaskStatus = await Promise.all(
            result.issues.map(async (issue) => {
              const existingTask = await db.query.tasks.findFirst({
                where: and(
                  eq(tasks.workspaceId, config.workspaceId),
                  eq(tasks.sourcePluginId, plugin.id),
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
        } catch (mcpError) {
          console.warn(`MCP call failed for workspace ${config.workspaceId}, showing workspace without issues:`, mcpError);
          // Still show the workspace, just without issues
        }

        allIssues.push({
          workspaceId: config.workspaceId,
          workspaceName: workspace.name,
          owner: parsed.owner,
          repo: parsed.repo,
          issues: issuesWithTaskStatus,
        });
      } catch (error) {
        console.error(`Failed to process workspace ${config.workspaceId}:`, error);
        // Continue with other workspaces
      }
    }

    return NextResponse.json({
      workspaces: allIssues,
      totalIssues: allIssues.reduce((sum, w) => sum + w.issues.length, 0),
      issuesWithTasks: allIssues.reduce(
        (sum, w) => sum + w.issues.filter((i) => i.hasTask).length,
        0
      ),
    });
  } catch (error) {
    console.error("Failed to fetch GitHub issues:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to fetch issues" },
      { status: 500 }
    );
  }
}
