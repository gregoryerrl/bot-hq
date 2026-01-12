import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, logs, tasks, workspaces, agentSessions, approvals } from "../../lib/db/index.js";
import { eq, desc } from "drizzle-orm";

export function registerMonitoringTools(server: McpServer) {
  // logs_get - Get recent logs
  server.tool(
    "logs_get",
    "Get recent logs, optionally filtered by task or type",
    {
      taskId: z.number().optional().describe("Filter by task ID"),
      type: z
        .enum(["agent", "error", "approval", "test", "health"])
        .optional()
        .describe("Filter by log type"),
      limit: z.number().optional().default(50).describe("Max number of logs to return"),
    },
    async ({ taskId, type, limit }) => {
      const conditions = [];
      if (taskId) conditions.push(eq(logs.taskId, taskId));
      if (type) conditions.push(eq(logs.type, type));

      const logList = await db
        .select({
          id: logs.id,
          type: logs.type,
          message: logs.message,
          taskId: logs.taskId,
          workspaceId: logs.workspaceId,
          createdAt: logs.createdAt,
        })
        .from(logs)
        .where(conditions.length > 0 ? conditions[0] : undefined)
        .orderBy(desc(logs.createdAt))
        .limit(limit || 50);

      // Enrich with task and workspace names
      const enrichedLogs = await Promise.all(
        logList.map(async (log) => {
          const task = log.taskId
            ? await db.query.tasks.findFirst({
                where: eq(tasks.id, log.taskId),
              })
            : null;
          const workspace = log.workspaceId
            ? await db.query.workspaces.findFirst({
                where: eq(workspaces.id, log.workspaceId),
              })
            : null;

          return {
            id: log.id,
            type: log.type,
            message: log.message,
            taskTitle: task?.title || null,
            workspaceName: workspace?.name || null,
            createdAt: log.createdAt?.toISOString(),
          };
        })
      );

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(enrichedLogs, null, 2),
          },
        ],
      };
    }
  );

  // status_overview - Dashboard overview
  server.tool(
    "status_overview",
    "Get dashboard overview - running agents, pending work, task counts by state",
    {},
    async () => {
      // Count agents by status
      const allSessions = await db.select().from(agentSessions);
      const runningAgents = allSessions.filter((s) => s.status === "running").length;

      // Count tasks by state
      const allTasks = await db.select().from(tasks);
      const taskCounts = {
        new: allTasks.filter((t) => t.state === "new").length,
        queued: allTasks.filter((t) => t.state === "queued").length,
        in_progress: allTasks.filter((t) => t.state === "in_progress").length,
        pending_review: allTasks.filter((t) => t.state === "pending_review").length,
        done: allTasks.filter((t) => t.state === "done").length,
      };

      // Count pending approvals
      const allApprovals = await db.select().from(approvals);
      const pendingApprovals = allApprovals.filter((a) => a.status === "pending").length;

      // Count workspaces
      const allWorkspaces = await db.select().from(workspaces);

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              {
                agents: {
                  running: runningAgents,
                  total: allSessions.length,
                },
                tasks: taskCounts,
                approvals: {
                  pending: pendingApprovals,
                },
                workspaces: {
                  total: allWorkspaces.length,
                },
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // workspace_sync - Sync GitHub issues (now handled by plugin)
  server.tool(
    "workspace_sync",
    "Sync GitHub issues for a workspace (or all workspaces). Note: GitHub sync is now handled by the GitHub plugin.",
    {
      workspaceId: z.number().optional().describe("Workspace ID (omit to sync all)"),
    },
    async ({ workspaceId }) => {
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: false,
              message: "GitHub sync is now handled by the GitHub plugin. Use the /api/plugins/github/sync endpoint or enable the GitHub plugin.",
              workspaceId: workspaceId || "all",
            }),
          },
        ],
      };
    }
  );

  // workspace_list - List all workspaces
  server.tool(
    "workspace_list",
    "List all configured workspaces",
    {},
    async () => {
      const workspaceList = await db.select().from(workspaces);

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              workspaceList.map((w) => ({
                id: w.id,
                name: w.name,
                repoPath: w.repoPath,
                createdAt: w.createdAt?.toISOString(),
              })),
              null,
              2
            ),
          },
        ],
      };
    }
  );
}
