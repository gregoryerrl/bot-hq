"use client";

import { use, useEffect, useState, useCallback, useRef } from "react";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { ArrowLeft } from "lucide-react";
import { FlowCanvas } from "@/components/diagrams/flow-canvas";
import type { Node, Edge } from "@xyflow/react";

interface DiagramResponse {
  id: number;
  title: string;
  projectId: number;
  template: string | null;
  createdAt: string;
  updatedAt: string;
  nodes: Node[];
  edges: Edge[];
  groups: { id: number; label: string; color: string }[];
}

export default function VisualizerPage({
  params,
}: {
  params: Promise<{ id: string; diagramId: string }>;
}) {
  const { id: projectId, diagramId } = use(params);
  const router = useRouter();
  const [diagram, setDiagram] = useState<DiagramResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [notFound, setNotFound] = useState(false);
  const previousPositions = useRef<Map<string, { x: number; y: number }>>(
    new Map()
  );

  const fetchDiagram = useCallback(async () => {
    try {
      const res = await fetch(`/api/diagrams/${diagramId}`);
      if (res.status === 404) {
        setNotFound(true);
        return;
      }
      if (!res.ok) return;
      const data: DiagramResponse = await res.json();
      setDiagram(data);

      // Store initial positions for change detection
      const posMap = new Map<string, { x: number; y: number }>();
      for (const node of data.nodes) {
        posMap.set(node.id, { x: node.position.x, y: node.position.y });
      }
      previousPositions.current = posMap;
    } finally {
      setLoading(false);
    }
  }, [diagramId]);

  useEffect(() => {
    fetchDiagram();
  }, [fetchDiagram]);

  const handlePositionsChange = useCallback(
    (nodes: Node[]) => {
      for (const node of nodes) {
        const prev = previousPositions.current.get(node.id);
        if (
          !prev ||
          prev.x !== node.position.x ||
          prev.y !== node.position.y
        ) {
          const nodeId = parseInt(node.id);
          if (isNaN(nodeId)) continue;

          previousPositions.current.set(node.id, {
            x: node.position.x,
            y: node.position.y,
          });

          fetch(`/api/diagrams/${diagramId}/nodes/${nodeId}`, {
            method: "PATCH",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              positionX: node.position.x,
              positionY: node.position.y,
            }),
          });
        }
      }
    },
    [diagramId]
  );

  const handleBack = () => {
    router.push(`/projects/${projectId}`);
  };

  if (loading) {
    return (
      <div className="flex flex-col h-full items-center justify-center">
        <p className="text-muted-foreground">Loading diagram...</p>
      </div>
    );
  }

  if (notFound || !diagram) {
    return (
      <div className="flex flex-col h-full items-center justify-center gap-4">
        <p className="text-muted-foreground">Diagram not found</p>
        <Button variant="ghost" onClick={handleBack}>
          <ArrowLeft className="h-4 w-4 mr-2" />
          Back to project
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-4 py-2 border-b">
        <Button variant="ghost" size="sm" onClick={handleBack}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <h1 className="text-lg font-semibold">{diagram.title}</h1>
      </div>
      <div className="flex-1">
        <FlowCanvas
          diagramId={diagram.id}
          initialNodes={diagram.nodes}
          initialEdges={diagram.edges}
          onPositionsChange={handlePositionsChange}
        />
      </div>
    </div>
  );
}
