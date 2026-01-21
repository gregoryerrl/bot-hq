import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, tasks, workspaces, logs } from "../../lib/db/index.js";
import { eq, desc } from "drizzle-orm";

export function registerTaskTools(server: McpServer) {
  // task_list - List tasks with filters
  server.tool(
    "task_list",
    "List tasks with optional filters by workspace or state",
    {
      workspaceId: z.number().optional().describe("Filter by workspace ID"),
      state: z
        .enum(["new", "queued", "in_progress", "needs_help", "done"])
        .optional()
        .describe("Filter by task state"),
    },
    async ({ workspaceId, state }) => {
      let query = db.select().from(tasks);

      const conditions = [];
      if (workspaceId) {
        conditions.push(eq(tasks.workspaceId, workspaceId));
      }
      if (state) {
        conditions.push(eq(tasks.state, state));
      }

      const taskList = await db
        .select({
          id: tasks.id,
          title: tasks.title,
          state: tasks.state,
          workspaceId: tasks.workspaceId,
          priority: tasks.priority,
          branchName: tasks.branchName,
          assignedAt: tasks.assignedAt,
          updatedAt: tasks.updatedAt,
        })
        .from(tasks)
        .where(conditions.length > 0 ? conditions[0] : undefined)
        .orderBy(desc(tasks.priority), desc(tasks.updatedAt))
        .limit(100);

      // Enrich with workspace names
      const enrichedTasks = await Promise.all(
        taskList.map(async (task) => {
          const workspace = await db.query.workspaces.findFirst({
            where: eq(workspaces.id, task.workspaceId),
          });
          return {
            ...task,
            workspaceName: workspace?.name || "Unknown",
            assignedAt: task.assignedAt?.toISOString(),
            updatedAt: task.updatedAt?.toISOString(),
          };
        })
      );

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(enrichedTasks, null, 2),
          },
        ],
      };
    }
  );

  // task_get - Get full task details
  server.tool(
    "task_get",
    "Get full details of a specific task",
    {
      taskId: z.number().describe("The task ID to retrieve"),
    },
    async ({ taskId }) => {
      const task = await db.query.tasks.findFirst({
        where: eq(tasks.id, taskId),
      });

      if (!task) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Task ${taskId} not found` }),
            },
          ],
        };
      }

      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, task.workspaceId),
      });

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              {
                id: task.id,
                title: task.title,
                description: task.description,
                state: task.state,
                workspaceId: task.workspaceId,
                workspaceName: workspace?.name || "Unknown",
                repoPath: workspace?.repoPath || "Unknown",
                priority: task.priority,
                branchName: task.branchName,
                assignedAt: task.assignedAt?.toISOString(),
                updatedAt: task.updatedAt?.toISOString(),
                iterationCount: task.iterationCount || 1,
                feedback: task.feedback || null,
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // task_create - Create a new task
  server.tool(
    "task_create",
    "Create a new task (with or without GitHub issue)",
    {
      workspaceId: z.number().describe("The workspace ID for this task"),
      title: z.string().describe("Task title"),
      description: z.string().describe("Task description"),
      priority: z.number().optional().default(0).describe("Task priority (higher = more important)"),
    },
    async ({ workspaceId, title, description, priority }) => {
      // Verify workspace exists
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

      const [newTask] = await db
        .insert(tasks)
        .values({
          workspaceId,
          title,
          description,
          priority: priority || 0,
          state: "new",
        })
        .returning();

      // Log the creation
      await db.insert(logs).values({
        workspaceId,
        taskId: newTask.id,
        type: "agent",
        message: `Task created: ${title}`,
      });

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              taskId: newTask.id,
              message: `Task created: ${title}`,
            }),
          },
        ],
      };
    }
  );

  // task_update - Update task properties
  server.tool(
    "task_update",
    "Update task properties like priority, state, or notes",
    {
      taskId: z.number().describe("The task ID to update"),
      priority: z.number().optional().describe("New priority"),
      state: z
        .enum(["new", "queued", "in_progress", "needs_help", "done"])
        .optional()
        .describe("New state"),
      notes: z.string().optional().describe("Additional notes to append to description"),
    },
    async ({ taskId, priority, state, notes }) => {
      const task = await db.query.tasks.findFirst({
        where: eq(tasks.id, taskId),
      });

      if (!task) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Task ${taskId} not found`,
              }),
            },
          ],
        };
      }

      const updates: Record<string, unknown> = { updatedAt: new Date() };
      if (priority !== undefined) updates.priority = priority;
      if (state !== undefined) updates.state = state;
      if (notes !== undefined) {
        updates.description = task.description
          ? `${task.description}\n\n---\nManager notes: ${notes}`
          : `Manager notes: ${notes}`;
      }

      await db.update(tasks).set(updates).where(eq(tasks.id, taskId));

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Task ${taskId} updated`,
            }),
          },
        ],
      };
    }
  );

  // task_assign - Assign a task (move to queued)
  server.tool(
    "task_assign",
    "Assign a task - moves it from 'new' to 'queued' state",
    {
      taskId: z.number().describe("The task ID to assign"),
    },
    async ({ taskId }) => {
      const task = await db.query.tasks.findFirst({
        where: eq(tasks.id, taskId),
      });

      if (!task) {
        return {
          content: [
            {
              type: "text" as const,
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
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Task ${taskId} is not in 'new' state (current: ${task.state})`,
              }),
            },
          ],
        };
      }

      await db
        .update(tasks)
        .set({
          state: "queued",
          assignedAt: new Date(),
          updatedAt: new Date(),
        })
        .where(eq(tasks.id, taskId));

      await db.insert(logs).values({
        workspaceId: task.workspaceId,
        taskId,
        type: "agent",
        message: `Task assigned and queued`,
      });

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Task ${taskId} assigned and moved to 'queued' state`,
            }),
          },
        ],
      };
    }
  );
}
