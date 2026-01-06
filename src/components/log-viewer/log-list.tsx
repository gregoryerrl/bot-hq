"use client";

import { useLogStream } from "@/hooks/use-log-stream";
import { LogEntry } from "./log-entry";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Trash2 } from "lucide-react";

export function LogList() {
  const { logs, connected, clearLogs } = useLogStream();

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between mb-4">
        <Badge variant={connected ? "default" : "destructive"}>
          {connected ? "Live" : "Disconnected"}
        </Badge>
        <Button variant="outline" size="sm" onClick={clearLogs}>
          <Trash2 className="h-4 w-4 mr-2" />
          Clear
        </Button>
      </div>
      <ScrollArea className="flex-1 border rounded-lg p-4">
        {logs.length === 0 ? (
          <div className="text-center text-muted-foreground py-8">
            Waiting for logs...
          </div>
        ) : (
          logs.map((log) => <LogEntry key={log.id} log={log} />)
        )}
      </ScrollArea>
    </div>
  );
}
