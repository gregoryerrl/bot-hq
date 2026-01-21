"use client";

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Monitor, Bot, ChevronRight } from "lucide-react";

interface LogSource {
  id: string;
  type: "server" | "agent" | "manager";
  name: string;
  status: "live" | "running";
  latestMessage: string | null;
  latestAt: string | null;
  sessionId?: number;
  taskId?: number;
  taskTitle?: string;
}

interface LogSourceCardProps {
  source: LogSource;
}

function formatRelativeTime(dateString: string | null): string {
  if (!dateString) return "";

  const date = new Date(dateString);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSecs = Math.floor(diffMs / 1000);
  const diffMins = Math.floor(diffSecs / 60);
  const diffHours = Math.floor(diffMins / 60);
  const diffDays = Math.floor(diffHours / 24);

  if (diffSecs < 60) return `${diffSecs}s ago`;
  if (diffMins < 60) return `${diffMins}m ago`;
  if (diffHours < 24) return `${diffHours}h ago`;
  return `${diffDays}d ago`;
}

export function LogSourceCard({ source }: LogSourceCardProps) {
  const href = source.type === "server"
    ? "/logs/server"
    : source.type === "manager"
    ? "/logs/manager"
    : `/logs/agent/${source.sessionId || source.taskId}`;

  const Icon = source.type === "server" ? Monitor : Bot;

  return (
    <Link href={href}>
      <Card className="p-4 hover:bg-muted/50 transition-colors cursor-pointer">
        <div className="flex items-start gap-3">
          <div className="flex-shrink-0 mt-0.5">
            <Icon className="h-5 w-5 text-muted-foreground" />
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 mb-1">
              <span className="font-medium">{source.name}</span>
              <Badge
                variant={source.status === "running" ? "default" : "secondary"}
                className="text-xs"
              >
                {source.status === "running" ? "Running" : "Live"}
              </Badge>
            </div>

            {source.taskTitle && (
              <p className="text-xs text-muted-foreground mb-1">
                Task: {source.taskTitle}
              </p>
            )}

            <p className="text-sm text-muted-foreground truncate">
              {source.latestMessage || "No logs yet"}
            </p>
          </div>

          <div className="flex-shrink-0 flex items-center gap-2">
            {source.latestAt && (
              <span className="text-xs text-muted-foreground">
                {formatRelativeTime(source.latestAt)}
              </span>
            )}
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          </div>
        </div>
      </Card>
    </Link>
  );
}
