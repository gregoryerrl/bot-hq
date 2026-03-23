import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { eq, and, or, like, desc } from "drizzle-orm";
import {
  db,
  diagrams,
  diagramNodes,
  diagramEdges,
  diagramGroups,
} from "../../lib/db/index.js";

export function registerDiagramTools(server: McpServer) {
  // diagram_list
  server.tool(
    "diagram_list",
    "List diagrams for a project",
    {
      projectId: z.number(),
    },
    async ({ projectId }) => {
      const result = await db
        .select()
        .from(diagrams)
        .where(eq(diagrams.projectId, projectId))
        .orderBy(desc(diagrams.updatedAt));

      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    }
  );

  // diagram_get
  server.tool(
    "diagram_get",
    "Get a diagram with all nodes, edges, and groups",
    {
      diagramId: z.number(),
    },
    async ({ diagramId }) => {
      const diagram = await db
        .select()
        .from(diagrams)
        .where(eq(diagrams.id, diagramId))
        .get();

      if (!diagram) {
        return {
          content: [
            { type: "text", text: `Diagram ${diagramId} not found` },
          ],
          isError: true,
        };
      }

      const nodes = await db
        .select()
        .from(diagramNodes)
        .where(eq(diagramNodes.diagramId, diagramId));

      const edges = await db
        .select()
        .from(diagramEdges)
        .where(eq(diagramEdges.diagramId, diagramId));

      const groups = await db
        .select()
        .from(diagramGroups)
        .where(eq(diagramGroups.diagramId, diagramId));

      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(
              { ...diagram, nodes, edges, groups },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // diagram_create
  server.tool(
    "diagram_create",
    "Create an empty diagram",
    {
      projectId: z.number(),
      title: z.string(),
      template: z.string().optional(),
    },
    async ({ projectId, title, template }) => {
      const result = await db
        .insert(diagrams)
        .values({ projectId, title, template })
        .returning();

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // diagram_delete
  server.tool(
    "diagram_delete",
    "Delete a diagram and all its related data",
    {
      diagramId: z.number(),
    },
    async ({ diagramId }) => {
      const existing = await db
        .select()
        .from(diagrams)
        .where(eq(diagrams.id, diagramId))
        .get();

      if (!existing) {
        return {
          content: [
            { type: "text", text: `Diagram ${diagramId} not found` },
          ],
          isError: true,
        };
      }

      await db.delete(diagrams).where(eq(diagrams.id, diagramId));

      return {
        content: [
          {
            type: "text",
            text: `Diagram "${existing.title}" (ID: ${diagramId}) deleted successfully`,
          },
        ],
      };
    }
  );

  // diagram_add_node
  server.tool(
    "diagram_add_node",
    "Add a node to a diagram",
    {
      diagramId: z.number(),
      label: z.string(),
      nodeType: z.string().optional(),
      description: z.string().optional(),
      metadata: z.record(z.string(), z.unknown()).optional(),
      groupId: z.number().optional(),
      positionX: z.number().optional(),
      positionY: z.number().optional(),
    },
    async ({ diagramId, label, nodeType, description, metadata, groupId, positionX, positionY }) => {
      const result = await db
        .insert(diagramNodes)
        .values({
          diagramId,
          label,
          nodeType: nodeType ?? "default",
          description,
          metadata: metadata ? JSON.stringify(metadata) : undefined,
          groupId,
          positionX: positionX ?? 0,
          positionY: positionY ?? 0,
        })
        .returning();

      await db
        .update(diagrams)
        .set({ updatedAt: new Date() })
        .where(eq(diagrams.id, diagramId));

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // diagram_update_node
  server.tool(
    "diagram_update_node",
    "Update a node's fields",
    {
      nodeId: z.number(),
      label: z.string().optional(),
      nodeType: z.string().optional(),
      description: z.string().optional(),
      metadata: z.record(z.string(), z.unknown()).optional(),
      groupId: z.number().nullable().optional(),
      positionX: z.number().optional(),
      positionY: z.number().optional(),
    },
    async ({ nodeId, ...fields }) => {
      const existing = await db
        .select()
        .from(diagramNodes)
        .where(eq(diagramNodes.id, nodeId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Node ${nodeId} not found` }],
          isError: true,
        };
      }

      const updates: Record<string, unknown> = {};
      if (fields.label !== undefined) updates.label = fields.label;
      if (fields.nodeType !== undefined) updates.nodeType = fields.nodeType;
      if (fields.description !== undefined)
        updates.description = fields.description;
      if (fields.metadata !== undefined)
        updates.metadata = JSON.stringify(fields.metadata);
      if (fields.groupId !== undefined) updates.groupId = fields.groupId;
      if (fields.positionX !== undefined) updates.positionX = fields.positionX;
      if (fields.positionY !== undefined) updates.positionY = fields.positionY;

      const result = await db
        .update(diagramNodes)
        .set(updates)
        .where(eq(diagramNodes.id, nodeId))
        .returning();

      await db
        .update(diagrams)
        .set({ updatedAt: new Date() })
        .where(eq(diagrams.id, existing.diagramId));

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // diagram_remove_node
  server.tool(
    "diagram_remove_node",
    "Delete a node and its connected edges",
    {
      nodeId: z.number(),
    },
    async ({ nodeId }) => {
      const existing = await db
        .select()
        .from(diagramNodes)
        .where(eq(diagramNodes.id, nodeId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Node ${nodeId} not found` }],
          isError: true,
        };
      }

      // Delete connected edges first
      await db
        .delete(diagramEdges)
        .where(
          or(
            eq(diagramEdges.sourceNodeId, nodeId),
            eq(diagramEdges.targetNodeId, nodeId)
          )
        );

      await db.delete(diagramNodes).where(eq(diagramNodes.id, nodeId));

      await db
        .update(diagrams)
        .set({ updatedAt: new Date() })
        .where(eq(diagrams.id, existing.diagramId));

      return {
        content: [
          {
            type: "text",
            text: `Node "${existing.label}" (ID: ${nodeId}) and connected edges deleted`,
          },
        ],
      };
    }
  );

  // diagram_add_edge
  server.tool(
    "diagram_add_edge",
    "Add an edge between two nodes",
    {
      diagramId: z.number(),
      sourceNodeId: z.number(),
      targetNodeId: z.number(),
      label: z.string().optional(),
    },
    async ({ diagramId, sourceNodeId, targetNodeId, label }) => {
      const result = await db
        .insert(diagramEdges)
        .values({ diagramId, sourceNodeId, targetNodeId, label })
        .returning();

      await db
        .update(diagrams)
        .set({ updatedAt: new Date() })
        .where(eq(diagrams.id, diagramId));

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // diagram_remove_edge
  server.tool(
    "diagram_remove_edge",
    "Delete an edge",
    {
      edgeId: z.number(),
    },
    async ({ edgeId }) => {
      const existing = await db
        .select()
        .from(diagramEdges)
        .where(eq(diagramEdges.id, edgeId))
        .get();

      if (!existing) {
        return {
          content: [{ type: "text", text: `Edge ${edgeId} not found` }],
          isError: true,
        };
      }

      await db.delete(diagramEdges).where(eq(diagramEdges.id, edgeId));

      return {
        content: [
          {
            type: "text",
            text: `Edge (ID: ${edgeId}) deleted successfully`,
          },
        ],
      };
    }
  );

  // diagram_add_group
  server.tool(
    "diagram_add_group",
    "Add a group to a diagram",
    {
      diagramId: z.number(),
      label: z.string(),
      color: z.string().optional(),
    },
    async ({ diagramId, label, color }) => {
      const result = await db
        .insert(diagramGroups)
        .values({ diagramId, label, color: color ?? "#6b7280" })
        .returning();

      await db
        .update(diagrams)
        .set({ updatedAt: new Date() })
        .where(eq(diagrams.id, diagramId));

      return {
        content: [{ type: "text", text: JSON.stringify(result[0], null, 2) }],
      };
    }
  );

  // diagram_bulk_add
  server.tool(
    "diagram_bulk_add",
    "Batch add groups, nodes, and edges with tempId resolution",
    {
      diagramId: z.number(),
      groups: z
        .array(
          z.object({
            tempId: z.string(),
            label: z.string(),
            color: z.string().optional(),
          })
        )
        .optional(),
      nodes: z
        .array(
          z.object({
            tempId: z.string(),
            label: z.string(),
            nodeType: z.string().optional(),
            description: z.string().optional(),
            metadata: z.record(z.string(), z.unknown()).optional(),
            groupTempId: z.string().optional(),
            positionX: z.number().optional(),
            positionY: z.number().optional(),
          })
        )
        .optional(),
      edges: z
        .array(
          z.object({
            sourceTempId: z.string(),
            targetTempId: z.string(),
            label: z.string().optional(),
          })
        )
        .optional(),
    },
    async ({ diagramId, groups, nodes, edges }) => {
      const groupIdMap: Record<string, number> = {};
      const nodeIdMap: Record<string, number> = {};
      let groupsCreated = 0;
      let nodesCreated = 0;
      let edgesCreated = 0;

      // Insert groups first
      if (groups) {
        for (const g of groups) {
          const result = await db
            .insert(diagramGroups)
            .values({
              diagramId,
              label: g.label,
              color: g.color ?? "#6b7280",
            })
            .returning();
          groupIdMap[g.tempId] = result[0].id;
          groupsCreated++;
        }
      }

      // Insert nodes (resolve groupTempId)
      if (nodes) {
        for (const n of nodes) {
          const groupId = n.groupTempId
            ? groupIdMap[n.groupTempId]
            : undefined;
          const result = await db
            .insert(diagramNodes)
            .values({
              diagramId,
              label: n.label,
              nodeType: n.nodeType ?? "default",
              description: n.description,
              metadata: n.metadata ? JSON.stringify(n.metadata) : undefined,
              groupId,
              positionX: n.positionX ?? 0,
              positionY: n.positionY ?? 0,
            })
            .returning();
          nodeIdMap[n.tempId] = result[0].id;
          nodesCreated++;
        }
      }

      // Insert edges (resolve sourceTempId/targetTempId)
      if (edges) {
        for (const e of edges) {
          const sourceNodeId = nodeIdMap[e.sourceTempId];
          const targetNodeId = nodeIdMap[e.targetTempId];
          if (sourceNodeId === undefined || targetNodeId === undefined) {
            continue; // skip edges with unresolved tempIds
          }
          await db.insert(diagramEdges).values({
            diagramId,
            sourceNodeId,
            targetNodeId,
            label: e.label,
          });
          edgesCreated++;
        }
      }

      await db
        .update(diagrams)
        .set({ updatedAt: new Date() })
        .where(eq(diagrams.id, diagramId));

      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(
              {
                groupsCreated,
                nodesCreated,
                edgesCreated,
                groupIdMap,
                nodeIdMap,
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // diagram_query
  server.tool(
    "diagram_query",
    "Search nodes in a diagram with combinable filters",
    {
      diagramId: z.number(),
      query: z.string().optional(),
      nodeType: z.string().optional(),
      groupId: z.number().optional(),
    },
    async ({ diagramId, query, nodeType, groupId }) => {
      const conditions = [eq(diagramNodes.diagramId, diagramId)];

      if (query) {
        const pattern = `%${query}%`;
        conditions.push(
          or(
            like(diagramNodes.label, pattern),
            like(diagramNodes.description, pattern)
          )!
        );
      }
      if (nodeType) {
        conditions.push(eq(diagramNodes.nodeType, nodeType));
      }
      if (groupId !== undefined) {
        conditions.push(eq(diagramNodes.groupId, groupId));
      }

      const result = await db
        .select()
        .from(diagramNodes)
        .where(and(...conditions));

      return {
        content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
      };
    }
  );

  // diagram_auto_layout
  server.tool(
    "diagram_auto_layout",
    "Auto-layout diagram nodes using BFS topological sort",
    {
      diagramId: z.number(),
    },
    async ({ diagramId }) => {
      const NODE_GAP = 300;
      const LAYER_GAP = 250;

      const nodes = await db
        .select()
        .from(diagramNodes)
        .where(eq(diagramNodes.diagramId, diagramId));

      const edges = await db
        .select()
        .from(diagramEdges)
        .where(eq(diagramEdges.diagramId, diagramId));

      if (nodes.length === 0) {
        return {
          content: [
            { type: "text", text: "No nodes to layout" },
          ],
        };
      }

      // Build adjacency and in-degree maps
      const adj = new Map<number, number[]>();
      const inDegree = new Map<number, number>();

      for (const node of nodes) {
        adj.set(node.id, []);
        inDegree.set(node.id, 0);
      }

      for (const edge of edges) {
        adj.get(edge.sourceNodeId)?.push(edge.targetNodeId);
        inDegree.set(
          edge.targetNodeId,
          (inDegree.get(edge.targetNodeId) ?? 0) + 1
        );
      }

      // BFS topological sort
      const layers: number[][] = [];
      const visited = new Set<number>();

      // Start with nodes that have no incoming edges
      let queue = nodes
        .filter((n) => (inDegree.get(n.id) ?? 0) === 0)
        .map((n) => n.id);

      while (queue.length > 0) {
        layers.push([...queue]);
        for (const id of queue) {
          visited.add(id);
        }

        const nextQueue: number[] = [];
        for (const id of queue) {
          for (const neighbor of adj.get(id) ?? []) {
            if (!visited.has(neighbor)) {
              inDegree.set(neighbor, (inDegree.get(neighbor) ?? 0) - 1);
              if ((inDegree.get(neighbor) ?? 0) <= 0 && !visited.has(neighbor)) {
                nextQueue.push(neighbor);
                visited.add(neighbor);
              }
            }
          }
        }
        queue = nextQueue;
      }

      // Add any remaining unvisited nodes (cycles) as a final layer
      const remaining = nodes
        .filter((n) => !visited.has(n.id))
        .map((n) => n.id);
      if (remaining.length > 0) {
        layers.push(remaining);
      }

      // Assign positions: horizontal layers, centered vertically
      for (let layerIdx = 0; layerIdx < layers.length; layerIdx++) {
        const layer = layers[layerIdx];
        const totalHeight = (layer.length - 1) * LAYER_GAP;
        const startY = -totalHeight / 2;

        for (let nodeIdx = 0; nodeIdx < layer.length; nodeIdx++) {
          const nodeId = layer[nodeIdx];
          const positionX = layerIdx * NODE_GAP;
          const positionY = startY + nodeIdx * LAYER_GAP;

          await db
            .update(diagramNodes)
            .set({ positionX, positionY })
            .where(eq(diagramNodes.id, nodeId));
        }
      }

      await db
        .update(diagrams)
        .set({ updatedAt: new Date() })
        .where(eq(diagrams.id, diagramId));

      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(
              {
                layersCount: layers.length,
                nodesLayouted: nodes.length,
                layers: layers.map((l, i) => ({
                  layer: i,
                  nodeIds: l,
                })),
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );
}
