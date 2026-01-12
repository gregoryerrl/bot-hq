"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerTaskTools = registerTaskTools;
const zod_1 = require("zod");
const index_js_1 = require("../../lib/db/index.js");
const drizzle_orm_1 = require("drizzle-orm");
function registerTaskTools(server) {
    // task_list - List tasks with filters
    server.tool("task_list", "List tasks with optional filters by workspace or state", {
        workspaceId: zod_1.z.number().optional().describe("Filter by workspace ID"),
        state: zod_1.z
            .enum(["new", "queued", "in_progress", "pending_review", "pr_created", "done"])
            .optional()
            .describe("Filter by task state"),
    }, async ({ workspaceId, state }) => {
        let query = index_js_1.db.select().from(index_js_1.tasks);
        const conditions = [];
        if (workspaceId) {
            conditions.push((0, drizzle_orm_1.eq)(index_js_1.tasks.workspaceId, workspaceId));
        }
        if (state) {
            conditions.push((0, drizzle_orm_1.eq)(index_js_1.tasks.state, state));
        }
        const taskList = await index_js_1.db
            .select({
            id: index_js_1.tasks.id,
            title: index_js_1.tasks.title,
            state: index_js_1.tasks.state,
            workspaceId: index_js_1.tasks.workspaceId,
            githubIssueNumber: index_js_1.tasks.githubIssueNumber,
            priority: index_js_1.tasks.priority,
            branchName: index_js_1.tasks.branchName,
            assignedAt: index_js_1.tasks.assignedAt,
            updatedAt: index_js_1.tasks.updatedAt,
        })
            .from(index_js_1.tasks)
            .where(conditions.length > 0 ? conditions[0] : undefined)
            .orderBy((0, drizzle_orm_1.desc)(index_js_1.tasks.priority), (0, drizzle_orm_1.desc)(index_js_1.tasks.updatedAt))
            .limit(100);
        // Enrich with workspace names
        const enrichedTasks = await Promise.all(taskList.map(async (task) => {
            const workspace = await index_js_1.db.query.workspaces.findFirst({
                where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, task.workspaceId),
            });
            return {
                ...task,
                workspaceName: workspace?.name || "Unknown",
                assignedAt: task.assignedAt?.toISOString(),
                updatedAt: task.updatedAt?.toISOString(),
            };
        }));
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify(enrichedTasks, null, 2),
                },
            ],
        };
    });
    // task_get - Get full task details
    server.tool("task_get", "Get full details of a specific task", {
        taskId: zod_1.z.number().describe("The task ID to retrieve"),
    }, async ({ taskId }) => {
        const task = await index_js_1.db.query.tasks.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, taskId),
        });
        if (!task) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({ error: `Task ${taskId} not found` }),
                    },
                ],
            };
        }
        const workspace = await index_js_1.db.query.workspaces.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, task.workspaceId),
        });
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
                        id: task.id,
                        title: task.title,
                        description: task.description,
                        state: task.state,
                        workspaceId: task.workspaceId,
                        workspaceName: workspace?.name || "Unknown",
                        repoPath: workspace?.repoPath || "Unknown",
                        githubIssueNumber: task.githubIssueNumber,
                        priority: task.priority,
                        branchName: task.branchName,
                        prUrl: task.prUrl,
                        assignedAt: task.assignedAt?.toISOString(),
                        updatedAt: task.updatedAt?.toISOString(),
                    }, null, 2),
                },
            ],
        };
    });
    // task_create - Create a new task
    server.tool("task_create", "Create a new task (with or without GitHub issue)", {
        workspaceId: zod_1.z.number().describe("The workspace ID for this task"),
        title: zod_1.z.string().describe("Task title"),
        description: zod_1.z.string().describe("Task description"),
        priority: zod_1.z.number().optional().default(0).describe("Task priority (higher = more important)"),
    }, async ({ workspaceId, title, description, priority }) => {
        // Verify workspace exists
        const workspace = await index_js_1.db.query.workspaces.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, workspaceId),
        });
        if (!workspace) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Workspace ${workspaceId} not found`,
                        }),
                    },
                ],
            };
        }
        const [newTask] = await index_js_1.db
            .insert(index_js_1.tasks)
            .values({
            workspaceId,
            title,
            description,
            priority: priority || 0,
            state: "new",
        })
            .returning();
        // Log the creation
        await index_js_1.db.insert(index_js_1.logs).values({
            workspaceId,
            taskId: newTask.id,
            type: "agent",
            message: `Task created: ${title}`,
        });
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
                        success: true,
                        taskId: newTask.id,
                        message: `Task created: ${title}`,
                    }),
                },
            ],
        };
    });
    // task_update - Update task properties
    server.tool("task_update", "Update task properties like priority, state, or notes", {
        taskId: zod_1.z.number().describe("The task ID to update"),
        priority: zod_1.z.number().optional().describe("New priority"),
        state: zod_1.z
            .enum(["new", "queued", "in_progress", "pending_review", "pr_created", "done"])
            .optional()
            .describe("New state"),
        notes: zod_1.z.string().optional().describe("Additional notes to append to description"),
    }, async ({ taskId, priority, state, notes }) => {
        const task = await index_js_1.db.query.tasks.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, taskId),
        });
        if (!task) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Task ${taskId} not found`,
                        }),
                    },
                ],
            };
        }
        const updates = { updatedAt: new Date() };
        if (priority !== undefined)
            updates.priority = priority;
        if (state !== undefined)
            updates.state = state;
        if (notes !== undefined) {
            updates.description = task.description
                ? `${task.description}\n\n---\nManager notes: ${notes}`
                : `Manager notes: ${notes}`;
        }
        await index_js_1.db.update(index_js_1.tasks).set(updates).where((0, drizzle_orm_1.eq)(index_js_1.tasks.id, taskId));
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
                        success: true,
                        message: `Task ${taskId} updated`,
                    }),
                },
            ],
        };
    });
    // task_assign - Assign a task (move to queued)
    server.tool("task_assign", "Assign a task - moves it from 'new' to 'queued' state", {
        taskId: zod_1.z.number().describe("The task ID to assign"),
    }, async ({ taskId }) => {
        const task = await index_js_1.db.query.tasks.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, taskId),
        });
        if (!task) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Task ${taskId} not found`,
                        }),
                    },
                ],
            };
        }
        if (task.state !== "new") {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Task ${taskId} is not in 'new' state (current: ${task.state})`,
                        }),
                    },
                ],
            };
        }
        await index_js_1.db
            .update(index_js_1.tasks)
            .set({
            state: "queued",
            assignedAt: new Date(),
            updatedAt: new Date(),
        })
            .where((0, drizzle_orm_1.eq)(index_js_1.tasks.id, taskId));
        await index_js_1.db.insert(index_js_1.logs).values({
            workspaceId: task.workspaceId,
            taskId,
            type: "agent",
            message: `Task assigned and queued`,
        });
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
                        success: true,
                        message: `Task ${taskId} assigned and moved to 'queued' state`,
                    }),
                },
            ],
        };
    });
}
