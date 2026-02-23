"use client";

import { useCallback, useMemo, useState } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  type Node,
  type Edge,
  type OnNodesChange,
  type OnEdgesChange,
  applyNodeChanges,
  applyEdgeChanges,
  MarkerType,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { FlowNode, type FlowNodeData } from "./flow-node";
import { NodeDetailDialog } from "./node-detail-dialog";

interface FlowCanvasProps {
  diagramId: number;
  initialNodes: Node[];
  initialEdges: Edge[];
  onPositionsChange?: (nodes: Node[]) => void;
}

const nodeTypes = { flowNode: FlowNode };

const defaultEdgeOptions = {
  animated: true,
  markerEnd: { type: MarkerType.ArrowClosed },
  style: { strokeWidth: 2 },
};

export function FlowCanvas({
  diagramId,
  initialNodes,
  initialEdges,
  onPositionsChange,
}: FlowCanvasProps) {
  const [nodes, setNodes] = useState<Node[]>(initialNodes);
  const [edges, setEdges] = useState<Edge[]>(initialEdges);
  const [selectedNode, setSelectedNode] = useState<{ id: string; data: FlowNodeData } | null>(null);

  const onNodesChange: OnNodesChange = useCallback(
    (changes) => {
      setNodes((nds) => {
        const updated = applyNodeChanges(changes, nds);
        const hasDragStop = changes.some((c) => c.type === "position" && c.dragging === false);
        if (hasDragStop && onPositionsChange) {
          onPositionsChange(updated);
        }
        return updated;
      });
    },
    [onPositionsChange]
  );

  const onEdgesChange: OnEdgesChange = useCallback(
    (changes) => setEdges((eds) => applyEdgeChanges(changes, eds)),
    []
  );

  const onNodeClick = useCallback(
    (_: React.MouseEvent, node: Node) => {
      setSelectedNode({ id: node.id, data: node.data as unknown as FlowNodeData });
    },
    []
  );

  const connectedNodes = useMemo(() => {
    if (!selectedNode) return { incoming: [], outgoing: [] };

    const incoming = edges
      .filter((e) => e.target === selectedNode.id)
      .map((e) => {
        const sourceNode = nodes.find((n) => n.id === e.source);
        return (sourceNode?.data as unknown as FlowNodeData)?.label || e.source;
      });

    const outgoing = edges
      .filter((e) => e.source === selectedNode.id)
      .map((e) => {
        const targetNode = nodes.find((n) => n.id === e.target);
        return (targetNode?.data as unknown as FlowNodeData)?.label || e.target;
      });

    return { incoming, outgoing };
  }, [selectedNode, edges, nodes]);

  return (
    <div className="w-full h-full">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={onNodeClick}
        nodeTypes={nodeTypes}
        defaultEdgeOptions={defaultEdgeOptions}
        fitView
        fitViewOptions={{ padding: 0.2 }}
        proOptions={{ hideAttribution: true }}
      >
        <Background />
        <Controls />
      </ReactFlow>

      <NodeDetailDialog
        open={!!selectedNode}
        onClose={() => setSelectedNode(null)}
        node={selectedNode}
        connectedNodes={connectedNodes}
      />
    </div>
  );
}
