"use client";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { type FlowNodeData, stringToColor } from "./flow-node";

interface NodeDetailDialogProps {
  open: boolean;
  onClose: () => void;
  node: { id: string; data: FlowNodeData } | null;
  connectedNodes: { incoming: string[]; outgoing: string[] };
}

export function NodeDetailDialog({ open, onClose, node, connectedNodes }: NodeDetailDialogProps) {
  if (!node) return null;

  const data = node.data;
  const dotColor = data.groupColor || stringToColor(data.nodeType || "default");

  // Separate files from other metadata keys
  const metadataEntries = Object.entries(data.metadata || {});
  const filesEntry = metadataEntries.find(([key]) => key === "files");
  const otherEntries = metadataEntries.filter(([key]) => key !== "files");

  const files = Array.isArray(filesEntry?.[1]) ? filesEntry![1] as { path: string; lineStart?: number; lineEnd?: number }[] : [];

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <div
              className="h-3 w-3 rounded-full flex-shrink-0"
              style={{ backgroundColor: dotColor }}
            />
            {data.label}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <Badge variant="outline">{data.nodeType}</Badge>
          </div>

          <div>
            <h4 className="text-sm font-medium mb-1">Description</h4>
            <p className="text-sm text-muted-foreground">{data.description}</p>
          </div>

          {files.length > 0 && (
            <div>
              <h4 className="text-sm font-medium mb-1">Files</h4>
              <div className="space-y-1">
                {files.map((f, i) => (
                  <p key={i} className="text-xs font-mono text-muted-foreground">
                    {f.path}
                    {f.lineStart ? `:${f.lineStart}` : ""}
                    {f.lineEnd ? `-${f.lineEnd}` : ""}
                  </p>
                ))}
              </div>
            </div>
          )}

          {otherEntries.map(([key, value]) => (
            <div key={key}>
              <h4 className="text-sm font-medium mb-1 capitalize">{key}</h4>
              {Array.isArray(value) && value.every((v) => typeof v === "string") ? (
                <div className="space-y-1">
                  {(value as string[]).map((item, i) => (
                    <pre
                      key={i}
                      className="text-xs bg-muted p-2 rounded overflow-x-auto"
                    >
                      {item}
                    </pre>
                  ))}
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">
                  {typeof value === "string" ? value : JSON.stringify(value)}
                </p>
              )}
            </div>
          ))}

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
        </div>
      </DialogContent>
    </Dialog>
  );
}
