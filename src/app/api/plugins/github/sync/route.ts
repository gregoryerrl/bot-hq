import { NextRequest, NextResponse } from "next/server";
import { db, tasks, pluginWorkspaceData, plugins } from "@/lib/db";
import { getMcpManager } from "@/lib/plugins";
import { eq, and } from "drizzle-orm";

interface GitHubIssue {
  number: number;
  title: string;
  body: string | null;
  url: string;
}

interface SyncResult {
  issues: GitHubIssue[];
}

export async function POST(request: NextRequest) {
  try {
    const { workspaceId } = await request.json();

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

    // Call MCP tool
    const manager = getMcpManager();
    const result = await manager.callTool("github", "github_sync_issues", {
      owner: config.owner,
      repo: config.repo,
    }) as SyncResult;

    // Create tasks for new issues
    let created = 0;
    for (const issue of result.issues) {
      // Check if task already exists for this issue
      const existing = await db.query.tasks.findFirst({
        where: and(
          eq(tasks.workspaceId, workspaceId),
          eq(tasks.sourcePluginId, plugin.id),
          eq(tasks.sourceRef, String(issue.number))
        ),
      });

      if (!existing) {
        await db.insert(tasks).values({
          workspaceId,
          sourcePluginId: plugin.id,
          sourceRef: String(issue.number),
          title: issue.title,
          description: issue.body || "",
          state: "new",
        });
        created++;
      }
    }

    return NextResponse.json({
      synced: result.issues.length,
      created,
    });
  } catch (error) {
    console.error("GitHub sync failed:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Sync failed" },
      { status: 500 }
    );
  }
}
