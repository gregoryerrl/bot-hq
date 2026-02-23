"use client";

import { useState, useEffect, useCallback, use } from "react";
import { Button } from "@/components/ui/button";
import { ArrowLeft } from "lucide-react";
import { FlowCanvas } from "@/components/diagrams/flow-canvas";
import type { Node, Edge } from "@xyflow/react";

interface DiagramData {
  id: number;
  title: string;
  workspaceId: number;
  flowData: string;
  createdAt: string;
  updatedAt: string;
}

export default function DiagramCanvasPage({
  params,
}: {
  params: Promise<{ id: string; diagramId: string }>;
}) {
  const { id: workspaceId, diagramId } = use(params);
  const [diagram, setDiagram] = useState<DiagramData | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    async function fetchDiagram() {
      try {
        const res = await fetch(`/api/diagrams/${diagramId}`);
        if (res.ok) {
          setDiagram(await res.json());
        }
      } catch (error) {
        console.error("Failed to fetch diagram:", error);
      } finally {
        setLoading(false);
      }
    }
    fetchDiagram();
  }, [diagramId]);

  const handlePositionsChange = useCallback(
    async (updatedNodes: Node[]) => {
      if (!diagram) return;

      try {
        const flowData = JSON.parse(diagram.flowData);
        flowData.nodes = updatedNodes.map((n) => ({
          ...flowData.nodes.find((fn: Node) => fn.id === n.id),
          position: n.position,
        }));

        await fetch(`/api/diagrams/${diagramId}`, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ flowData }),
        });
      } catch (error) {
        console.error("Failed to save positions:", error);
      }
    },
    [diagram, diagramId]
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Loading diagram...
      </div>
    );
  }

  if (!diagram) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4">
        <p className="text-muted-foreground">Diagram not found</p>
        <Button
          variant="outline"
          size="sm"
          onClick={() => window.location.assign(`/workspaces/${workspaceId}/diagram`)}
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back to diagrams
        </Button>
      </div>
    );
  }

  let nodes: Node[] = [];
  let edges: Edge[] = [];
  let parseError = false;
  try {
    const parsed = JSON.parse(diagram.flowData);
    nodes = (parsed.nodes || []).map((n: Node) => ({
      ...n,
      type: "flowNode",
    }));
    edges = parsed.edges || [];
  } catch {
    parseError = true;
  }

  if (parseError || nodes.length === 0) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-3 px-4 py-3 border-b">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => window.location.assign(`/workspaces/${workspaceId}/diagram`)}
          >
            <ArrowLeft className="h-4 w-4 mr-1" />
            Back
          </Button>
          <h1 className="text-lg font-semibold">{diagram.title}</h1>
        </div>
        <div className="flex-1 flex items-center justify-center">
          <p className="text-muted-foreground">
            {parseError ? "Diagram data is invalid and cannot be rendered." : "This diagram has no nodes yet."}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 px-4 py-3 border-b">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => window.location.assign(`/workspaces/${workspaceId}/diagram`)}
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <h1 className="text-lg font-semibold">{diagram.title}</h1>
      </div>
      <div className="flex-1">
        <FlowCanvas
          diagramId={diagram.id}
          initialNodes={nodes}
          initialEdges={edges}
          onPositionsChange={handlePositionsChange}
        />
      </div>
    </div>
  );
}
