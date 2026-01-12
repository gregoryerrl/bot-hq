"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerMonitoringTools = registerMonitoringTools;
const zod_1 = require("zod");
const index_js_1 = require("../../lib/db/index.js");
const drizzle_orm_1 = require("drizzle-orm");
const index_js_2 = require("../../lib/sync/index.js");
function registerMonitoringTools(server) {
    // logs_get - Get recent logs
    server.tool("logs_get", "Get recent logs, optionally filtered by task or type", {
        taskId: zod_1.z.number().optional().describe("Filter by task ID"),
        type: zod_1.z
            .enum(["agent", "error", "approval", "sync"])
            .optional()
            .describe("Filter by log type"),
        limit: zod_1.z.number().optional().default(50).describe("Max number of logs to return"),
    }, async ({ taskId, type, limit }) => {
        const conditions = [];
        if (taskId)
            conditions.push((0, drizzle_orm_1.eq)(index_js_1.logs.taskId, taskId));
        if (type)
            conditions.push((0, drizzle_orm_1.eq)(index_js_1.logs.type, type));
        const logList = await index_js_1.db
            .select({
            id: index_js_1.logs.id,
            type: index_js_1.logs.type,
            message: index_js_1.logs.message,
            taskId: index_js_1.logs.taskId,
            workspaceId: index_js_1.logs.workspaceId,
            createdAt: index_js_1.logs.createdAt,
        })
            .from(index_js_1.logs)
            .where(conditions.length > 0 ? conditions[0] : undefined)
            .orderBy((0, drizzle_orm_1.desc)(index_js_1.logs.createdAt))
            .limit(limit || 50);
        // Enrich with task and workspace names
        const enrichedLogs = await Promise.all(logList.map(async (log) => {
            const task = log.taskId
                ? await index_js_1.db.query.tasks.findFirst({
                    where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, log.taskId),
                })
                : null;
            const workspace = log.workspaceId
                ? await index_js_1.db.query.workspaces.findFirst({
                    where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, log.workspaceId),
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
        }));
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify(enrichedLogs, null, 2),
                },
            ],
        };
    });
    // status_overview - Dashboard overview
    server.tool("status_overview", "Get dashboard overview - running agents, pending work, task counts by state", {}, async () => {
        // Count agents by status
        const allSessions = await index_js_1.db.select().from(index_js_1.agentSessions);
        const runningAgents = allSessions.filter((s) => s.status === "running").length;
        // Count tasks by state
        const allTasks = await index_js_1.db.select().from(index_js_1.tasks);
        const taskCounts = {
            new: allTasks.filter((t) => t.state === "new").length,
            queued: allTasks.filter((t) => t.state === "queued").length,
            in_progress: allTasks.filter((t) => t.state === "in_progress").length,
            pending_review: allTasks.filter((t) => t.state === "pending_review").length,
            pr_created: allTasks.filter((t) => t.state === "pr_created").length,
            done: allTasks.filter((t) => t.state === "done").length,
        };
        // Count pending approvals
        const allApprovals = await index_js_1.db.select().from(index_js_1.approvals);
        const pendingApprovals = allApprovals.filter((a) => a.status === "pending").length;
        // Count workspaces
        const allWorkspaces = await index_js_1.db.select().from(index_js_1.workspaces);
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
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
                    }, null, 2),
                },
            ],
        };
    });
    // workspace_sync - Sync GitHub issues
    server.tool("workspace_sync", "Sync GitHub issues for a workspace (or all workspaces)", {
        workspaceId: zod_1.z.number().optional().describe("Workspace ID (omit to sync all)"),
    }, async ({ workspaceId }) => {
        try {
            if (workspaceId) {
                await (0, index_js_2.syncWorkspaceIssues)(workspaceId);
                return {
                    content: [
                        {
                            type: "text",
                            text: JSON.stringify({
                                success: true,
                                message: `Synced workspace ${workspaceId}`,
                            }),
                        },
                    ],
                };
            }
            else {
                await (0, index_js_2.syncAllWorkspaces)();
                return {
                    content: [
                        {
                            type: "text",
                            text: JSON.stringify({
                                success: true,
                                message: "Synced all workspaces",
                            }),
                        },
                    ],
                };
            }
        }
        catch (error) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Sync failed: ${error}`,
                        }),
                    },
                ],
            };
        }
    });
    // workspace_list - List all workspaces
    server.tool("workspace_list", "List all configured workspaces", {}, async () => {
        const workspaceList = await index_js_1.db.select().from(index_js_1.workspaces);
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify(workspaceList.map((w) => ({
                        id: w.id,
                        name: w.name,
                        repoPath: w.repoPath,
                        githubRemote: w.githubRemote,
                        createdAt: w.createdAt?.toISOString(),
                    })), null, 2),
                },
            ],
        };
    });
}
