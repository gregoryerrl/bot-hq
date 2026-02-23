"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Wrench, Clock } from "lucide-react";
import { formatDistanceToNow } from "date-fns";

interface FlowNode {
  id: string;
  data: {
    layer: "ux" | "frontend" | "backend" | "database";
    activeTask?: { taskId: number; state: string } | null;
  };
}

interface FlowCardProps {
  id: number;
  title: string;
  flowData: string;
  updatedAt: string | Date;
  onClick: (id: number) => void;
}

const LAYER_COLORS: Record<string, string> = {
  ux: "bg-blue-500",
  frontend: "bg-green-500",
  backend: "bg-red-500",
  database: "bg-purple-500",
};

export function FlowCard({ id, title, flowData, updatedAt, onClick }: FlowCardProps) {
  let nodes: FlowNode[] = [];
  try {
    const parsed = JSON.parse(flowData);
    nodes = parsed.nodes || [];
  } catch {
    // Invalid JSON, show empty
  }

  const layerCounts: Record<string, number> = {};
  let hasWorking = false;
  let hasPending = false;

  for (const node of nodes) {
    const layer = node.data?.layer || "backend";
    layerCounts[layer] = (layerCounts[layer] || 0) + 1;

    if (node.data?.activeTask) {
      if (node.data.activeTask.state === "in_progress") hasWorking = true;
      if (node.data.activeTask.state === "pending") hasPending = true;
    }
  }

  return (
    <Card
      className="p-4 cursor-pointer hover:bg-muted/50 transition-colors"
      onClick={() => onClick(id)}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
          <h3 className="font-medium truncate">{title}</h3>
          <div className="flex items-center gap-1.5 mt-2">
            {["ux", "frontend", "backend", "database"].map((layer) =>
              layerCounts[layer] ? (
                <div key={layer} className="flex items-center gap-1">
                  <div className={`h-2.5 w-2.5 rounded-full ${LAYER_COLORS[layer]}`} />
                  <span className="text-xs text-muted-foreground">{layerCounts[layer]}</span>
                </div>
              ) : null
            )}
          </div>
          <p className="text-xs text-muted-foreground mt-2">
            Updated {formatDistanceToNow(new Date(updatedAt), { addSuffix: true })}
          </p>
        </div>
        <div className="flex items-center gap-1">
          {hasWorking && (
            <Badge variant="outline" className="text-orange-500 border-orange-500">
              <Wrench className="h-3 w-3 mr-1" />
              Working
            </Badge>
          )}
          {hasPending && (
            <Badge variant="outline" className="text-yellow-500 border-yellow-500">
              <Clock className="h-3 w-3 mr-1" />
              Pending
            </Badge>
          )}
        </div>
      </div>
    </Card>
  );
}
