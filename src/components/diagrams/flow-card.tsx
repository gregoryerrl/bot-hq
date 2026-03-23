"use client";

import { Card } from "@/components/ui/card";
import { formatDistanceToNow } from "date-fns";

interface FlowCardProps {
  id: number;
  title: string;
  nodeCount: number;
  edgeCount: number;
  groupCount: number;
  updatedAt: string | Date;
  onClick: (id: number) => void;
}

export function FlowCard({ id, title, nodeCount, edgeCount, groupCount, updatedAt, onClick }: FlowCardProps) {
  return (
    <Card
      className="p-4 cursor-pointer hover:bg-muted/50 transition-colors"
      onClick={() => onClick(id)}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
          <h3 className="font-medium truncate">{title}</h3>
          <p className="text-sm text-muted-foreground mt-2">
            {nodeCount} node{nodeCount !== 1 ? "s" : ""}, {edgeCount} edge{edgeCount !== 1 ? "s" : ""}
            {groupCount > 0 && (
              <span> &middot; {groupCount} group{groupCount !== 1 ? "s" : ""}</span>
            )}
          </p>
          <p className="text-xs text-muted-foreground mt-2">
            Updated {formatDistanceToNow(new Date(updatedAt), { addSuffix: true })}
          </p>
        </div>
      </div>
    </Card>
  );
}
