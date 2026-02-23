import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, diagrams, workspaces } from "../../lib/db/index.js";
import { eq } from "drizzle-orm";

export function registerDiagramTools(server: McpServer) {
  server.tool(
    "diagram_list",
    "List flow diagrams for a workspace",
    {
      workspaceId: z.number().describe("The workspace ID"),
    },
    async ({ workspaceId }) => {
      const diagramList = await db
        .select({
          id: diagrams.id,
          title: diagrams.title,
          workspaceId: diagrams.workspaceId,
          updatedAt: diagrams.updatedAt,
        })
        .from(diagrams)
        .where(eq(diagrams.workspaceId, workspaceId));

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(diagramList, null, 2),
          },
        ],
      };
    }
  );

  server.tool(
    "diagram_get",
    "Get full details of a specific diagram including flow data",
    {
      diagramId: z.number().describe("The diagram ID"),
    },
    async ({ diagramId }) => {
      const diagram = await db.query.diagrams.findFirst({
        where: eq(diagrams.id, diagramId),
      });

      if (!diagram) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Diagram ${diagramId} not found` }),
            },
          ],
        };
      }

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              {
                id: diagram.id,
                title: diagram.title,
                workspaceId: diagram.workspaceId,
                flowData: JSON.parse(diagram.flowData),
                createdAt: diagram.createdAt,
                updatedAt: diagram.updatedAt,
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  server.tool(
    "diagram_create",
    "Create a new flow diagram for a workspace",
    {
      workspaceId: z.number().describe("The workspace ID"),
      title: z.string().describe("Diagram title (e.g., 'User Registration')"),
      flowData: z.string().describe("JSON string of { nodes: [...], edges: [...] } in React Flow format"),
    },
    async ({ workspaceId, title, flowData }) => {
      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, workspaceId),
      });

      if (!workspace) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Workspace ${workspaceId} not found` }),
            },
          ],
        };
      }

      try {
        JSON.parse(flowData);
      } catch {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: "Invalid flowData JSON" }),
            },
          ],
        };
      }

      const [newDiagram] = await db
        .insert(diagrams)
        .values({ workspaceId, title, flowData })
        .returning();

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              diagramId: newDiagram.id,
              message: `Diagram "${title}" created`,
            }),
          },
        ],
      };
    }
  );

  server.tool(
    "diagram_update",
    "Update an existing diagram's flow data or title",
    {
      diagramId: z.number().describe("The diagram ID to update"),
      title: z.string().optional().describe("New title"),
      flowData: z.string().optional().describe("Updated JSON string of { nodes: [...], edges: [...] }"),
    },
    async ({ diagramId, title, flowData }) => {
      const existing = await db.query.diagrams.findFirst({
        where: eq(diagrams.id, diagramId),
      });

      if (!existing) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Diagram ${diagramId} not found` }),
            },
          ],
        };
      }

      const updates: Record<string, unknown> = { updatedAt: new Date() };
      if (title) updates.title = title;
      if (flowData) {
        try {
          JSON.parse(flowData);
        } catch {
          return {
            content: [
              {
                type: "text" as const,
                text: JSON.stringify({ error: "Invalid flowData JSON" }),
              },
            ],
          };
        }
        updates.flowData = flowData;
      }

      await db.update(diagrams).set(updates).where(eq(diagrams.id, diagramId));

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Diagram ${diagramId} updated`,
            }),
          },
        ],
      };
    }
  );
}
