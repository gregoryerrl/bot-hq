"use client";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Wrench, Clock, ExternalLink } from "lucide-react";
import type { FlowNodeData } from "./flow-node";

interface NodeDetailDialogProps {
  open: boolean;
  onClose: () => void;
  node: { id: string; data: FlowNodeData } | null;
  connectedNodes: { incoming: string[]; outgoing: string[] };
}

const LAYER_LABELS: Record<string, { label: string; color: string }> = {
  ux: { label: "UX", color: "bg-blue-500" },
  frontend: { label: "Frontend", color: "bg-green-500" },
  backend: { label: "Backend", color: "bg-red-500" },
  database: { label: "Database", color: "bg-purple-500" },
};

export function NodeDetailDialog({ open, onClose, node, connectedNodes }: NodeDetailDialogProps) {
  if (!node) return null;

  const data = node.data;
  const layer = LAYER_LABELS[data.layer] || LAYER_LABELS.backend;

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <div className={`h-3 w-3 rounded-full ${layer.color}`} />
            {data.label}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <Badge variant="outline">{layer.label}</Badge>
          </div>

          <div>
            <h4 className="text-sm font-medium mb-1">Description</h4>
            <p className="text-sm text-muted-foreground">{data.description}</p>
          </div>

          {data.files?.length > 0 && (
            <div>
              <h4 className="text-sm font-medium mb-1">Files</h4>
              <div className="space-y-1">
                {data.files.map((f, i) => (
                  <p key={i} className="text-xs font-mono text-muted-foreground">
                    {f.path}
                    {f.lineStart ? `:${f.lineStart}` : ""}
                    {f.lineEnd ? `-${f.lineEnd}` : ""}
                  </p>
                ))}
              </div>
            </div>
          )}

          {data.codeSnippets && data.codeSnippets.length > 0 && (
            <div>
              <h4 className="text-sm font-medium mb-1">Code</h4>
              {data.codeSnippets.map((snippet, i) => (
                <pre
                  key={i}
                  className="text-xs bg-muted p-2 rounded overflow-x-auto mt-1"
                >
                  {snippet}
                </pre>
              ))}
            </div>
          )}

          {(connectedNodes.incoming.length > 0 || connectedNodes.outgoing.length > 0) && (
            <div>
              <h4 className="text-sm font-medium mb-1">Connections</h4>
              {connectedNodes.incoming.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  From: {connectedNodes.incoming.join(", ")}
                </p>
              )}
              {connectedNodes.outgoing.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  To: {connectedNodes.outgoing.join(", ")}
                </p>
              )}
            </div>
          )}

          {data.activeTask && (
            <div className="flex items-center gap-2">
              {data.activeTask.state === "in_progress" ? (
                <Badge variant="outline" className="text-orange-500 border-orange-500">
                  <Wrench className="h-3 w-3 mr-1" />
                  Task #{data.activeTask.taskId} — Working
                </Badge>
              ) : (
                <Badge variant="outline" className="text-yellow-500 border-yellow-500">
                  <Clock className="h-3 w-3 mr-1" />
                  Task #{data.activeTask.taskId} — Pending Review
                </Badge>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  if (data.activeTask!.state === "in_progress") {
                    window.location.assign(`/logs?taskId=${data.activeTask!.taskId}`);
                  } else {
                    window.location.assign("/pending");
                  }
                }}
              >
                <ExternalLink className="h-3 w-3 mr-1" />
                {data.activeTask.state === "in_progress" ? "View Logs" : "Review Changes"}
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
