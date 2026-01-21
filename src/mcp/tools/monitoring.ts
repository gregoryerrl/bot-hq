import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, logs, tasks, workspaces, gitRemotes } from "../../lib/db/index.js";
import { eq, desc, and } from "drizzle-orm";

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
      // Count tasks by state
      const allTasks = await db.select().from(tasks);
      const taskCounts = {
        new: allTasks.filter((t) => t.state === "new").length,
        queued: allTasks.filter((t) => t.state === "queued").length,
        in_progress: allTasks.filter((t) => t.state === "in_progress").length,
        needs_help: allTasks.filter((t) => t.state === "needs_help").length,
        done: allTasks.filter((t) => t.state === "done").length,
      };

      // Count workspaces
      const allWorkspaces = await db.select().from(workspaces);

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              {
                manager: {
                  status: "pending_implementation",
                },
                tasks: taskCounts,
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

  // workspace_sync - Sync issues from git remote
  server.tool(
    "workspace_sync",
    "Sync issues for a workspace from its configured git remote. Use the /git-remote page to configure remotes.",
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
              message: "Issue sync is now handled by the Git Remote feature. Use the /git-remote page or /api/git-remote/issues endpoint.",
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
                linkedDirs: w.linkedDirs,
                buildCommand: w.buildCommand,
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

  // workspace_create - Create a new workspace
  server.tool(
    "workspace_create",
    "Create a new workspace for a repository",
    {
      name: z.string().describe("Workspace name (usually the repo/project name)"),
      repoPath: z.string().describe("Absolute path to the repository"),
      linkedDirs: z.string().optional().describe("JSON array of linked directories (e.g., for theme repos that build into another dir)"),
      buildCommand: z.string().optional().describe("Build command (e.g., 'npm run build')"),
    },
    async ({ name, repoPath, linkedDirs, buildCommand }) => {
      // Check if workspace with same name exists
      const existing = await db.query.workspaces.findFirst({
        where: eq(workspaces.name, name),
      });

      if (existing) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Workspace '${name}' already exists`,
              }),
            },
          ],
        };
      }

      const [newWorkspace] = await db
        .insert(workspaces)
        .values({
          name,
          repoPath,
          linkedDirs: linkedDirs || null,
          buildCommand: buildCommand || null,
          createdAt: new Date(),
        })
        .returning();

      await db.insert(logs).values({
        workspaceId: newWorkspace.id,
        type: "agent",
        message: `Workspace created: ${name} -> ${repoPath}`,
      });

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              workspaceId: newWorkspace.id,
              message: `Workspace '${name}' created`,
            }),
          },
        ],
      };
    }
  );

  // workspace_delete - Delete a workspace
  server.tool(
    "workspace_delete",
    "Delete a workspace (does NOT delete the actual files, only the bot-hq record)",
    {
      workspaceId: z.number().describe("The workspace ID to delete"),
    },
    async ({ workspaceId }) => {
      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, workspaceId),
      });

      if (!workspace) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Workspace ${workspaceId} not found`,
              }),
            },
          ],
        };
      }

      // Check if there are associated tasks
      const associatedTasks = await db
        .select()
        .from(tasks)
        .where(eq(tasks.workspaceId, workspaceId));

      if (associatedTasks.length > 0) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Cannot delete workspace with ${associatedTasks.length} associated tasks. Delete or reassign tasks first.`,
              }),
            },
          ],
        };
      }

      await db.delete(workspaces).where(eq(workspaces.id, workspaceId));

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Workspace '${workspace.name}' deleted`,
            }),
          },
        ],
      };
    }
  );

  // workspace_update - Update workspace configuration
  server.tool(
    "workspace_update",
    "Update workspace configuration (linked dirs, build command, etc.)",
    {
      workspaceId: z.number().describe("The workspace ID to update"),
      linkedDirs: z.string().optional().describe("JSON array of linked directories"),
      buildCommand: z.string().optional().describe("Build command"),
    },
    async ({ workspaceId, linkedDirs, buildCommand }) => {
      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, workspaceId),
      });

      if (!workspace) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Workspace ${workspaceId} not found`,
              }),
            },
          ],
        };
      }

      const updates: Record<string, unknown> = {};
      if (linkedDirs !== undefined) updates.linkedDirs = linkedDirs;
      if (buildCommand !== undefined) updates.buildCommand = buildCommand;

      if (Object.keys(updates).length === 0) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: "No updates provided",
              }),
            },
          ],
        };
      }

      await db.update(workspaces).set(updates).where(eq(workspaces.id, workspaceId));

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Workspace '${workspace.name}' updated`,
            }),
          },
        ],
      };
    }
  );

  // github_list_all_issues - List GitHub issues from all workspaces with git remotes
  server.tool(
    "github_list_all_issues",
    "List GitHub issues from all configured workspaces in one view",
    {},
    async () => {
      // Get all git remotes with GitHub provider
      const remotes = await db
        .select()
        .from(gitRemotes)
        .where(eq(gitRemotes.provider, "github"));

      if (remotes.length === 0) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                message: "No GitHub remotes configured. Use the /git-remote page to add a remote.",
                workspaces: [],
                totalIssues: 0,
              }),
            },
          ],
        };
      }

      // Note: Actual issue fetching would require GitHub API calls
      // This is a placeholder that returns the configured remotes
      const workspaceRemotes = await Promise.all(
        remotes
          .filter(r => r.workspaceId && r.owner && r.repo)
          .map(async (remote) => {
            const workspace = await db.query.workspaces.findFirst({
              where: eq(workspaces.id, remote.workspaceId!),
            });

            return {
              workspaceId: remote.workspaceId,
              workspaceName: workspace?.name || "Unknown",
              owner: remote.owner,
              repo: remote.repo,
              remoteId: remote.id,
            };
          })
      );

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              {
                message: "Use /api/git-remote/issues to fetch actual issues",
                configuredRemotes: workspaceRemotes,
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // github_create_task_from_issue - Create a task from a GitHub issue
  server.tool(
    "github_create_task_from_issue",
    "Create a bot-hq task from a GitHub issue",
    {
      workspaceId: z.number().describe("The workspace ID"),
      issueNumber: z.number().describe("The GitHub issue number"),
      priority: z.number().optional().default(0).describe("Task priority"),
    },
    async ({ workspaceId, issueNumber, priority }) => {
      // Get the git remote for this workspace
      const remote = await db.query.gitRemotes.findFirst({
        where: and(
          eq(gitRemotes.workspaceId, workspaceId),
          eq(gitRemotes.provider, "github")
        ),
      });

      if (!remote) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: "No GitHub remote configured for this workspace. Use /git-remote to configure.",
              }),
            },
          ],
        };
      }

      if (!remote.owner || !remote.repo) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: "GitHub remote owner/repo not configured",
              }),
            },
          ],
        };
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
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Task already exists for issue #${issueNumber}`,
                taskId: existingTask.id,
              }),
            },
          ],
        };
      }

      // Create the task (actual issue details would need GitHub API)
      const [newTask] = await db
        .insert(tasks)
        .values({
          workspaceId,
          sourceRemoteId: remote.id,
          sourceRef: String(issueNumber),
          title: `GitHub Issue #${issueNumber}`,
          description: `Issue from ${remote.owner}/${remote.repo}`,
          priority: priority || 0,
          state: "new",
        })
        .returning();

      await db.insert(logs).values({
        workspaceId,
        taskId: newTask.id,
        type: "agent",
        message: `Task created from GitHub issue #${issueNumber}`,
      });

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              taskId: newTask.id,
              message: `Task created from GitHub issue #${issueNumber}. Use /api/git-remote/issues/sync for full issue details.`,
            }),
          },
        ],
      };
    }
  );
}
