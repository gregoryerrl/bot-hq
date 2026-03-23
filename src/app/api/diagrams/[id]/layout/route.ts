import { NextRequest, NextResponse } from "next/server";
import { db, diagrams, diagramNodes, diagramEdges } from "@/lib/db";
import { eq } from "drizzle-orm";

const NODE_GAP = 300;
const LAYER_GAP = 250;

export async function POST(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const diagramId = Number(id);

  // 1. Query all nodes and edges
  const nodes = await db
    .select()
    .from(diagramNodes)
    .where(eq(diagramNodes.diagramId, diagramId));

  const edges = await db
    .select()
    .from(diagramEdges)
    .where(eq(diagramEdges.diagramId, diagramId));

  if (nodes.length === 0) {
    return NextResponse.json({ success: true, layers: 0, nodes: 0 });
  }

  // 2. Build adjacency list and in-degree map
  const adjacency = new Map<number, number[]>();
  const inDegree = new Map<number, number>();

  for (const node of nodes) {
    adjacency.set(node.id, []);
    inDegree.set(node.id, 0);
  }

  for (const edge of edges) {
    const neighbors = adjacency.get(edge.sourceNodeId);
    if (neighbors) {
      neighbors.push(edge.targetNodeId);
    }
    inDegree.set(edge.targetNodeId, (inDegree.get(edge.targetNodeId) ?? 0) + 1);
  }

  // 3. BFS topological sort into layers
  const layers: number[][] = [];
  const visited = new Set<number>();

  // Start with nodes that have in-degree 0
  let queue = nodes
    .filter((n) => (inDegree.get(n.id) ?? 0) === 0)
    .map((n) => n.id);

  while (queue.length > 0) {
    layers.push([...queue]);
    for (const nodeId of queue) {
      visited.add(nodeId);
    }

    const nextQueue: number[] = [];
    for (const nodeId of queue) {
      const neighbors = adjacency.get(nodeId) ?? [];
      for (const neighbor of neighbors) {
        inDegree.set(neighbor, (inDegree.get(neighbor) ?? 0) - 1);
        if ((inDegree.get(neighbor) ?? 0) === 0 && !visited.has(neighbor)) {
          nextQueue.push(neighbor);
        }
      }
    }
    queue = nextQueue;
  }

  // 4. Place unvisited nodes (cycles/disconnected) in a final layer
  const unvisited = nodes.filter((n) => !visited.has(n.id)).map((n) => n.id);
  if (unvisited.length > 0) {
    layers.push(unvisited);
  }

  // 5. Assign positions
  const positionUpdates: { id: number; positionX: number; positionY: number }[] = [];

  for (let layerIdx = 0; layerIdx < layers.length; layerIdx++) {
    const layer = layers[layerIdx];
    const layerWidth = (layer.length - 1) * NODE_GAP;
    const startX = -(layerWidth / 2);

    for (let nodeIdx = 0; nodeIdx < layer.length; nodeIdx++) {
      positionUpdates.push({
        id: layer[nodeIdx],
        positionX: startX + nodeIdx * NODE_GAP,
        positionY: layerIdx * LAYER_GAP,
      });
    }
  }

  // 6. Update each node's position in DB
  for (const update of positionUpdates) {
    await db
      .update(diagramNodes)
      .set({ positionX: update.positionX, positionY: update.positionY })
      .where(eq(diagramNodes.id, update.id));
  }

  // 7. Update diagram updatedAt
  await db
    .update(diagrams)
    .set({ updatedAt: new Date() })
    .where(eq(diagrams.id, diagramId));

  return NextResponse.json({
    success: true,
    layers: layers.length,
    nodes: positionUpdates.length,
  });
}
