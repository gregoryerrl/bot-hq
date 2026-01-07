"use client";

import { useLogStream } from "@/hooks/use-log-stream";
import { LogEntry } from "./log-entry";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Trash2, ArrowLeft } from "lucide-react";
import Link from "next/link";

interface LogDetailProps {
  title: string;
  source: "server" | "agent";
  sessionId?: number;
  subtitle?: string;
}

export function LogDetail({ title, source, sessionId, subtitle }: LogDetailProps) {
  const { logs, connected, clearLogs } = useLogStream({ source, sessionId });

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-4 mb-4">
        <Link href="/logs">
          <Button variant="ghost" size="sm">
            <ArrowLeft className="h-4 w-4 mr-2" />
            Back
          </Button>
        </Link>
        <div className="flex-1">
          <h2 className="font-semibold">{title}</h2>
          {subtitle && (
            <p className="text-sm text-muted-foreground">{subtitle}</p>
          )}
        </div>
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
