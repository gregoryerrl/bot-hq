import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { eq, sql } from "drizzle-orm";
import { db, projects, tasks, diagrams } from "../../lib/db/index.js";

export function registerSummaryTools(server: McpServer) {
  // summary
  server.tool(
    "summary",
    "Get a summary of a project or all active projects with task counts",
    {
      projectId: z.number().optional(),
    },
    async ({ projectId }) => {
      if (projectId !== undefined) {
        // Single project summary
        const project = await db
          .select()
          .from(projects)
          .where(eq(projects.id, projectId))
          .get();

        if (!project) {
          return {
            content: [
              { type: "text", text: `Project ${projectId} not found` },
            ],
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

        const blockedTasks = await db
          .select()
          .from(tasks)
          .where(
            sql`${tasks.projectId} = ${projectId} AND ${tasks.state} = 'blocked'`
          );

        return {
          content: [
            {
              type: "text",
              text: JSON.stringify(
                {
                  name: project.name,
                  status: project.status,
                  taskCounts: Object.fromEntries(
                    taskCounts.map((tc) => [tc.state, tc.count])
                  ),
                  diagramCount: diagramCount.count,
                  blockedTasks: blockedTasks.map((t) => ({
                    id: t.id,
                    title: t.title,
                  })),
                },
                null,
                2
              ),
            },
          ],
        };
      }

      // All active projects summary
      const activeProjects = await db
        .select()
        .from(projects)
        .where(eq(projects.status, "active"));

      const summaries = [];

      for (const project of activeProjects) {
        const taskCounts = await db
          .select({
            state: tasks.state,
            count: sql<number>`count(*)`,
          })
          .from(tasks)
          .where(eq(tasks.projectId, project.id))
          .groupBy(tasks.state);

        summaries.push({
          id: project.id,
          name: project.name,
          taskCounts: Object.fromEntries(
            taskCounts.map((tc) => [tc.state, tc.count])
          ),
        });
      }

      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(summaries, null, 2),
          },
        ],
      };
    }
  );
}
