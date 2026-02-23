"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { formatDistanceToNow } from "date-fns";

interface LogEntry {
  id: number;
  type: string;
  message: string;
  createdAt: string | Date;
  workspaceName?: string;
  taskTitle?: string;
}

const typeBadgeColors: Record<string, string> = {
  agent: "bg-blue-500",
  test: "bg-purple-500",
  approval: "bg-green-500",
  error: "bg-red-500",
  health: "bg-gray-500",
};

interface RecentActivityProps {
  logs: LogEntry[];
}

export function RecentActivity({ logs }: RecentActivityProps) {
  return (
    <Card className="p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Recent Activity</h3>
        <a
          href="/logs"
          onClick={(e) => {
            e.preventDefault();
            window.location.assign("/logs");
          }}
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          View all
        </a>
      </div>
      {logs.length === 0 ? (
        <p className="text-sm text-muted-foreground py-4 text-center">
          No recent activity
        </p>
      ) : (
        <div className="space-y-2">
          {logs.map((log) => (
            <div
              key={log.id}
              className="flex items-start gap-2 py-2 border-b last:border-0"
            >
              <Badge
                variant="secondary"
                className={`${typeBadgeColors[log.type] || "bg-gray-500"} text-white text-xs shrink-0 mt-0.5`}
              >
                {log.type}
              </Badge>
              <p className="text-sm flex-1 line-clamp-1">{log.message}</p>
              <span className="text-xs text-muted-foreground shrink-0">
                {formatDistanceToNow(new Date(log.createdAt), { addSuffix: true })}
              </span>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
