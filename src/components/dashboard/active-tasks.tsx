"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Task } from "@/lib/db/schema";

const stateColors: Record<string, string> = {
  in_progress: "bg-orange-500",
  needs_help: "bg-red-500",
};

const stateLabels: Record<string, string> = {
  in_progress: "In Progress",
  needs_help: "Needs Help",
};

interface ActiveTasksProps {
  tasks: Array<Task & { workspaceName?: string }>;
}

export function ActiveTasks({ tasks }: ActiveTasksProps) {
  return (
    <Card className="p-4">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Active Tasks</h3>
        <a
          href="/taskboard"
          onClick={(e) => {
            e.preventDefault();
            window.location.assign("/taskboard");
          }}
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          View all
        </a>
      </div>
      {tasks.length === 0 ? (
        <p className="text-sm text-muted-foreground py-4 text-center">
          No active tasks
        </p>
      ) : (
        <div className="space-y-2">
          {tasks.map((task) => (
            <div
              key={task.id}
              className="flex items-center gap-2 py-2 border-b last:border-0"
            >
              <Badge
                variant="secondary"
                className={`${stateColors[task.state] || "bg-gray-500"} text-white text-xs shrink-0`}
              >
                {stateLabels[task.state] || task.state}
              </Badge>
              <span className="text-sm truncate flex-1">{task.title}</span>
              {task.workspaceName && (
                <Badge variant="outline" className="text-xs shrink-0">
                  {task.workspaceName}
                </Badge>
              )}
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
