"use client";

import { memo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { Wrench, Clock } from "lucide-react";

export interface FlowNodeData {
  label: string;
  layer: "ux" | "frontend" | "backend" | "database";
  description: string;
  files: { path: string; lineStart?: number; lineEnd?: number }[];
  codeSnippets?: string[];
  activeTask?: { taskId: number; state: string } | null;
  [key: string]: unknown;
}

const LAYER_STYLES: Record<string, { border: string; bg: string; label: string; color: string }> = {
  ux: { border: "border-l-blue-500", bg: "bg-blue-500/5", label: "UX", color: "text-blue-500" },
  frontend: { border: "border-l-green-500", bg: "bg-green-500/5", label: "Frontend", color: "text-green-500" },
  backend: { border: "border-l-red-500", bg: "bg-red-500/5", label: "Backend", color: "text-red-500" },
  database: { border: "border-l-purple-500", bg: "bg-purple-500/5", label: "Database", color: "text-purple-500" },
};

function FlowNodeComponent({ data }: NodeProps) {
  const [showTooltip, setShowTooltip] = useState(false);
  const nodeData = data as unknown as FlowNodeData;
  const style = LAYER_STYLES[nodeData.layer] || LAYER_STYLES.backend;
  const activeTask = nodeData.activeTask;

  return (
    <div
      className={`relative rounded-lg border-l-4 border bg-card shadow-sm min-w-[180px] max-w-[240px] ${style.border} ${style.bg}`}
      onMouseEnter={() => setShowTooltip(true)}
      onMouseLeave={() => setShowTooltip(false)}
    >
      <Handle type="target" position={Position.Left} className="!bg-muted-foreground" />

      <div className="px-3 py-2">
        <div className="flex items-center justify-between gap-1">
          <span className={`text-[10px] font-medium uppercase ${style.color}`}>
            {style.label}
          </span>
          {activeTask?.state === "in_progress" && (
            <Wrench className="h-3 w-3 text-orange-500" />
          )}
          {activeTask?.state === "pending" && (
            <Clock className="h-3 w-3 text-yellow-500" />
          )}
        </div>
        <p className="text-sm font-medium mt-1 leading-tight">{nodeData.label}</p>
      </div>

      <Handle type="source" position={Position.Right} className="!bg-muted-foreground" />

      {showTooltip && (
        <div className="absolute z-50 top-full left-0 mt-2 w-72 p-3 rounded-lg border bg-popover text-popover-foreground shadow-lg">
          <p className="text-sm">{nodeData.description}</p>
          {nodeData.files?.length > 0 && (
            <div className="mt-2">
              <p className="text-xs font-medium text-muted-foreground mb-1">Files:</p>
              {nodeData.files.map((f, i) => (
                <p key={i} className="text-xs font-mono text-muted-foreground">
                  {f.path}
                  {f.lineStart ? `:${f.lineStart}` : ""}
                  {f.lineEnd ? `-${f.lineEnd}` : ""}
                </p>
              ))}
            </div>
          )}
          {activeTask && (
            <p className="text-xs mt-2 text-orange-500">
              Task #{activeTask.taskId} — {activeTask.state}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

export const FlowNode = memo(FlowNodeComponent);
