"use client";

import { memo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";

export interface FlowNodeData {
  label: string;
  nodeType: string;
  description: string;
  metadata: Record<string, unknown>;
  groupId?: number;
  groupColor?: string;
  [key: string]: unknown;
}

export function stringToColor(str: string): string {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = str.charCodeAt(i) + ((hash << 5) - hash);
  }
  const hue = Math.abs(hash) % 360;
  return `hsl(${hue}, 65%, 55%)`;
}

function FlowNodeComponent({ data }: NodeProps) {
  const [showTooltip, setShowTooltip] = useState(false);
  const nodeData = data as unknown as FlowNodeData;
  const accentColor = nodeData.groupColor || stringToColor(nodeData.nodeType || "default");

  const files = Array.isArray(nodeData.metadata?.files) ? nodeData.metadata.files as { path: string; lineStart?: number; lineEnd?: number }[] : [];

  return (
    <div
      className="relative rounded-lg border bg-card shadow-sm min-w-[180px] max-w-[240px]"
      style={{ borderLeftWidth: 4, borderLeftColor: accentColor, backgroundColor: `${accentColor}08` }}
      onMouseEnter={() => setShowTooltip(true)}
      onMouseLeave={() => setShowTooltip(false)}
    >
      <Handle type="target" position={Position.Left} className="!bg-muted-foreground" />

      <div className="px-3 py-2">
        <div className="flex items-center justify-between gap-1">
          <span
            className="text-[10px] font-medium uppercase"
            style={{ color: accentColor }}
          >
            {nodeData.nodeType}
          </span>
        </div>
        <p className="text-sm font-medium mt-1 leading-tight">{nodeData.label}</p>
      </div>

      <Handle type="source" position={Position.Right} className="!bg-muted-foreground" />

      {showTooltip && (
        <div className="absolute z-50 top-full left-0 mt-2 w-72 p-3 rounded-lg border bg-popover text-popover-foreground shadow-lg">
          <p className="text-sm">{nodeData.description}</p>
          {files.length > 0 && (
            <div className="mt-2">
              <p className="text-xs font-medium text-muted-foreground mb-1">Files:</p>
              {files.map((f, i) => (
                <p key={i} className="text-xs font-mono text-muted-foreground">
                  {f.path}
                  {f.lineStart ? `:${f.lineStart}` : ""}
                  {f.lineEnd ? `-${f.lineEnd}` : ""}
                </p>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export const FlowNode = memo(FlowNodeComponent);
