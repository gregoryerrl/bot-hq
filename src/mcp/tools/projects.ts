import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { eq, like, or, desc, sql } from "drizzle-orm";
import { db, projects, tasks, diagrams } from "../../lib/db/index.js";

export function registerProjectTools(server: McpServer) {
  // project_list
  server.tool(
    "project_list",
    "List all projects, optionally filtered by status",
    {
      status: z.enum(["active", "archived"]).optional(),
    },
    async ({ status }) => {
      const result = status
        ? await db
            .select()
            .from(projects)
            .where(eq(projects.status, status))
            .orderBy(desc(projects.updatedAt))
        : await db
            .select()
            .from(projects)
            .orderBy(desc(projects.updatedAt));
      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    }
  );

  // project_get
  server.tool(
    "project_get",
    "Get a project by ID with task counts and diagram count",
    {
      projectId: z.number(),
    },
    async ({ projectId }) => {
      const project = await db
        .select()
        .from(projects)
        .where(eq(projects.id, projectId))
        .get();

      if (!project) {
        return {
          content: [{ type: "text", text: `Project ${projectId} not found` }],
          isError: true,
        };
      }

      const taskCounts = await db
        .select({
          state: tasks.state,
          count: sql<number>`count(*)`,
        })
        .from(tasks)
        .where(eq(tasks.projectId, projectId))
        .groupBy(tasks.state);

      const [diagramCount] = await db
        .select({ count: sql<number>`count(*)` })
        .from(diagrams)
        .where(eq(diagrams.projectId, projectId));

      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(
              {
                ...project,
                taskCounts: Object.fromEntries(
                  taskCounts.map((tc) => [tc.state, tc.count])
                ),
                diagramCount: diagramCount.count,
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // project_create
  server.tool(
    "project_create",
    "Create a new project",
    {
      name: z.string(),
      description: z.string().optional(),
      repoPath: z.string().optional(),
      notes: z.string().optional(),
    },
    async ({ name, description, repoPath, notes }) => {
      const result = await db
        .insert(projects)
        .values({ name, description, repoPath, notes })
        .returning();

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // project_update
  server.tool(
    "project_update",
    "Update an existing project",
    {
      projectId: z.number(),
      name: z.string().optional(),
      description: z.string().optional(),
      repoPath: z.string().optional(),
      status: z.enum(["active", "archived"]).optional(),
      notes: z.string().optional(),
    },
    async ({ projectId, ...fields }) => {
      const existing = await db
        .select()
        .from(projects)
        .where(eq(projects.id, projectId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Project ${projectId} not found` }],
          isError: true,
        };
      }

      const updates: Record<string, unknown> = { updatedAt: new Date() };
      if (fields.name !== undefined) updates.name = fields.name;
      if (fields.description !== undefined)
        updates.description = fields.description;
      if (fields.repoPath !== undefined) updates.repoPath = fields.repoPath;
      if (fields.status !== undefined) updates.status = fields.status;
      if (fields.notes !== undefined) updates.notes = fields.notes;

      const result = await db
        .update(projects)
        .set(updates)
        .where(eq(projects.id, projectId))
        .returning();

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // project_delete
  server.tool(
    "project_delete",
    "Delete a project and all its related data",
    {
      projectId: z.number(),
    },
    async ({ projectId }) => {
      const existing = await db
        .select()
        .from(projects)
        .where(eq(projects.id, projectId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Project ${projectId} not found` }],
          isError: true,
        };
      }

      await db.delete(projects).where(eq(projects.id, projectId));

      return {
        content: [
          {
            type: "text",
            text: `Project "${existing.name}" (ID: ${projectId}) deleted successfully`,
          },
        ],
      };
    }
  );

  // project_search
  server.tool(
    "project_search",
    "Search projects by name or description",
    {
      query: z.string(),
    },
    async ({ query }) => {
      const pattern = `%${query}%`;
      const result = await db
        .select()
        .from(projects)
        .where(
          or(
            like(projects.name, pattern),
            like(projects.description, pattern)
          )
        )
        .orderBy(desc(projects.updatedAt));

      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    }
  );
}
