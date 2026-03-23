import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { eq, like, or, and, desc, asc, isNull, sql } from "drizzle-orm";
import {
  db,
  tasks,
  taskNotes,
  taskDependencies,
} from "../../lib/db/index.js";

export function registerTaskTools(server: McpServer) {
  // task_list
  server.tool(
    "task_list",
    "List tasks for a project with optional filters",
    {
      projectId: z.number(),
      state: z
        .enum(["todo", "in_progress", "done", "blocked"])
        .optional(),
      parentTaskId: z.number().nullable().optional(),
      priority: z.number().optional(),
    },
    async ({ projectId, state, parentTaskId, priority }) => {
      const conditions = [eq(tasks.projectId, projectId)];

      if (state) {
        conditions.push(eq(tasks.state, state));
      }
      if (parentTaskId === null) {
        conditions.push(isNull(tasks.parentTaskId));
      } else if (parentTaskId !== undefined) {
        conditions.push(eq(tasks.parentTaskId, parentTaskId));
      }
      if (priority !== undefined) {
        conditions.push(eq(tasks.priority, priority));
      }

      const result = await db
        .select()
        .from(tasks)
        .where(and(...conditions))
        .orderBy(asc(tasks.order), desc(tasks.createdAt));

      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    }
  );

  // task_get
  server.tool(
    "task_get",
    "Get a task by ID with subtasks, notes, and dependencies",
    {
      taskId: z.number(),
    },
    async ({ taskId }) => {
      const task = await db
        .select()
        .from(tasks)
        .where(eq(tasks.id, taskId))
        .get();

      if (!task) {
        return {
          content: [{ type: "text", text: `Task ${taskId} not found` }],
          isError: true,
        };
      }

      const subtasks = await db
        .select()
        .from(tasks)
        .where(eq(tasks.parentTaskId, taskId))
        .orderBy(asc(tasks.order), desc(tasks.createdAt));

      const notes = await db
        .select()
        .from(taskNotes)
        .where(eq(taskNotes.taskId, taskId))
        .orderBy(desc(taskNotes.createdAt));

      const dependencies = await db
        .select()
        .from(taskDependencies)
        .where(eq(taskDependencies.taskId, taskId));

      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(
              { ...task, subtasks, notes, dependencies },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // task_create
  server.tool(
    "task_create",
    "Create a new task",
    {
      projectId: z.number(),
      title: z.string(),
      description: z.string().optional(),
      parentTaskId: z.number().optional(),
      priority: z.number().min(0).max(3).optional(),
      tags: z.array(z.string()).optional(),
      dueDate: z.string().optional(),
    },
    async ({ projectId, title, description, parentTaskId, priority, tags, dueDate }) => {
      const result = await db
        .insert(tasks)
        .values({
          projectId,
          title,
          description,
          parentTaskId,
          priority,
          tags: tags ? JSON.stringify(tags) : undefined,
          dueDate: dueDate ? new Date(dueDate) : undefined,
        })
        .returning();

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // task_update
  server.tool(
    "task_update",
    "Update an existing task",
    {
      taskId: z.number(),
      title: z.string().optional(),
      description: z.string().optional(),
      state: z
        .enum(["todo", "in_progress", "done", "blocked"])
        .optional(),
      priority: z.number().min(0).max(3).optional(),
      tags: z.array(z.string()).optional(),
      dueDate: z.string().nullable().optional(),
    },
    async ({ taskId, ...fields }) => {
      const existing = await db
        .select()
        .from(tasks)
        .where(eq(tasks.id, taskId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Task ${taskId} not found` }],
          isError: true,
        };
      }

      const updates: Record<string, unknown> = { updatedAt: new Date() };
      if (fields.title !== undefined) updates.title = fields.title;
      if (fields.description !== undefined)
        updates.description = fields.description;
      if (fields.state !== undefined) updates.state = fields.state;
      if (fields.priority !== undefined) updates.priority = fields.priority;
      if (fields.tags !== undefined)
        updates.tags = JSON.stringify(fields.tags);
      if (fields.dueDate !== undefined)
        updates.dueDate =
          fields.dueDate === null ? null : new Date(fields.dueDate);

      const result = await db
        .update(tasks)
        .set(updates)
        .where(eq(tasks.id, taskId))
        .returning();

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // task_delete
  server.tool(
    "task_delete",
    "Delete a task",
    {
      taskId: z.number(),
    },
    async ({ taskId }) => {
      const existing = await db
        .select()
        .from(tasks)
        .where(eq(tasks.id, taskId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Task ${taskId} not found` }],
          isError: true,
        };
      }

      await db.delete(tasks).where(eq(tasks.id, taskId));

      return {
        content: [
          {
            type: "text",
            text: `Task "${existing.title}" (ID: ${taskId}) deleted successfully`,
          },
        ],
      };
    }
  );

  // task_move
  server.tool(
    "task_move",
    "Move a task to a different parent or reorder it",
    {
      taskId: z.number(),
      parentTaskId: z.number().nullable().optional(),
      order: z.number().optional(),
    },
    async ({ taskId, parentTaskId, order }) => {
      const existing = await db
        .select()
        .from(tasks)
        .where(eq(tasks.id, taskId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Task ${taskId} not found` }],
          isError: true,
        };
      }

      const updates: Record<string, unknown> = { updatedAt: new Date() };
      if (parentTaskId !== undefined) updates.parentTaskId = parentTaskId;
      if (order !== undefined) updates.order = order;

      const result = await db
        .update(tasks)
        .set(updates)
        .where(eq(tasks.id, taskId))
        .returning();

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // task_add_note
  server.tool(
    "task_add_note",
    "Add a note to a task",
    {
      taskId: z.number(),
      content: z.string(),
    },
    async ({ taskId, content }) => {
      const existing = await db
        .select()
        .from(tasks)
        .where(eq(tasks.id, taskId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Task ${taskId} not found` }],
          isError: true,
        };
      }

      const result = await db
        .insert(taskNotes)
        .values({ taskId, content })
        .returning();

      return {
        content: [
          { type: "text", text: JSON.stringify(result[0], null, 2) },
        ],
      };
    }
  );

  // task_add_dependency
  server.tool(
    "task_add_dependency",
    "Add a dependency between tasks",
    {
      taskId: z.number(),
      dependsOnTaskId: z.number(),
    },
    async ({ taskId, dependsOnTaskId }) => {
      const task = await db
        .select()
        .from(tasks)
        .where(eq(tasks.id, taskId))
        .get();

      if (!task) {
        return {
          content: [{ type: "text", text: `Task ${taskId} not found` }],
          isError: true,
        };
      }

      const dependsOn = await db
        .select()
        .from(tasks)
        .where(eq(tasks.id, dependsOnTaskId))
        .get();

      if (!dependsOn) {
        return {
          content: [
            {
              type: "text",
              text: `Dependency task ${dependsOnTaskId} not found`,
            },
          ],
          isError: true,
        };
      }

      const result = await db
        .insert(taskDependencies)
        .values({ taskId, dependsOnTaskId })
        .returning();

      return {
        content: [
          { type: "text", text: JSON.stringify(result[0], null, 2) },
        ],
      };
    }
  );

  // task_search
  server.tool(
    "task_search",
    "Search tasks by title or description across all projects",
    {
      query: z.string(),
    },
    async ({ query }) => {
      const pattern = `%${query}%`;
      const result = await db
        .select()
        .from(tasks)
        .where(
          or(like(tasks.title, pattern), like(tasks.description, pattern))
        )
        .orderBy(desc(tasks.updatedAt));

      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    }
  );
}
