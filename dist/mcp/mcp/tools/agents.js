"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerAgentTools = registerAgentTools;
const zod_1 = require("zod");
const index_js_1 = require("../../lib/db/index.js");
const drizzle_orm_1 = require("drizzle-orm");
const claude_code_js_1 = require("../../lib/agents/claude-code.js");
// Track running agents
const runningAgents = new Map();
function registerAgentTools(server) {
    // agent_list - List all agent sessions
    server.tool("agent_list", "List all agent sessions with their status, workspace, and task info", {}, async () => {
        const sessions = await index_js_1.db
            .select({
            id: index_js_1.agentSessions.id,
            status: index_js_1.agentSessions.status,
            pid: index_js_1.agentSessions.pid,
            workspaceId: index_js_1.agentSessions.workspaceId,
            taskId: index_js_1.agentSessions.taskId,
            startedAt: index_js_1.agentSessions.startedAt,
            lastActivityAt: index_js_1.agentSessions.lastActivityAt,
        })
            .from(index_js_1.agentSessions)
            .orderBy((0, drizzle_orm_1.desc)(index_js_1.agentSessions.startedAt))
            .limit(50);
        // Enrich with workspace and task names
        const enrichedSessions = await Promise.all(sessions.map(async (session) => {
            const workspace = await index_js_1.db.query.workspaces.findFirst({
                where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, session.workspaceId),
            });
            const task = session.taskId
                ? await index_js_1.db.query.tasks.findFirst({
                    where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, session.taskId),
                })
                : null;
            return {
                id: session.id,
                status: session.status,
                pid: session.pid,
                workspaceName: workspace?.name || "Unknown",
                taskId: session.taskId,
                taskTitle: task?.title || "No task",
                startedAt: session.startedAt?.toISOString(),
                lastActivityAt: session.lastActivityAt?.toISOString(),
            };
        }));
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify(enrichedSessions, null, 2),
                },
            ],
        };
    });
    // agent_start - Start an agent on a task
    server.tool("agent_start", "Start a Claude Code agent to work on a specific task", {
        taskId: zod_1.z.number().describe("The task ID to start working on"),
    }, async ({ taskId }) => {
        // Check if agent already running for this task
        const existingSession = await index_js_1.db.query.agentSessions.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.agentSessions.taskId, taskId),
        });
        if (existingSession?.status === "running") {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Agent already running for task ${taskId}`,
                        }),
                    },
                ],
            };
        }
        // Start the agent
        const agent = await (0, claude_code_js_1.startAgentForTask)(taskId);
        if (!agent) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Failed to start agent - task ${taskId} not found or no workspace`,
                        }),
                    },
                ],
            };
        }
        runningAgents.set(taskId, agent);
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
                        success: true,
                        message: `Agent started for task ${taskId}`,
                    }),
                },
            ],
        };
    });
    // agent_stop - Stop a running agent
    server.tool("agent_stop", "Stop an agent that is currently working on a task", {
        taskId: zod_1.z.number().describe("The task ID to stop"),
    }, async ({ taskId }) => {
        // Try in-memory agent first
        const agent = runningAgents.get(taskId);
        if (agent) {
            await agent.stop();
            runningAgents.delete(taskId);
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: true,
                            message: `Agent stopped for task ${taskId}`,
                        }),
                    },
                ],
            };
        }
        // Try to find by PID in database
        const session = await index_js_1.db.query.agentSessions.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.agentSessions.taskId, taskId),
        });
        if (session?.pid && session.status === "running") {
            try {
                process.kill(session.pid, "SIGTERM");
                await index_js_1.db
                    .update(index_js_1.agentSessions)
                    .set({ status: "stopped" })
                    .where((0, drizzle_orm_1.eq)(index_js_1.agentSessions.id, session.id));
                return {
                    content: [
                        {
                            type: "text",
                            text: JSON.stringify({
                                success: true,
                                message: `Agent stopped (PID: ${session.pid})`,
                            }),
                        },
                    ],
                };
            }
            catch (error) {
                return {
                    content: [
                        {
                            type: "text",
                            text: JSON.stringify({
                                success: false,
                                message: `Failed to stop agent: ${error}`,
                            }),
                        },
                    ],
                };
            }
        }
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
                        success: false,
                        message: `No running agent found for task ${taskId}`,
                    }),
                },
            ],
        };
    });
}
