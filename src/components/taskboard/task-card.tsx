"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Play, RotateCcw } from "lucide-react";
import { Task } from "@/lib/db/schema";
import { PluginTaskBadges } from "@/components/plugins/plugin-task-badges";
import { PluginTaskActions } from "@/components/plugins/plugin-task-actions";

interface TaskCardProps {
  task: Task & { workspaceName?: string };
  onAssign: (taskId: number) => void;
  onStartTask: (taskId: number) => void;
  onRetry?: (taskId: number) => void;
}

const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  in_progress: "bg-orange-500",
  needs_help: "bg-red-500",
  done: "bg-green-700",
};

const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  in_progress: "In Progress",
  needs_help: "Needs Help",
  done: "Done",
};

export function TaskCard({ task, onAssign, onStartTask, onRetry }: TaskCardProps) {
  return (
    <Card className="p-3 md:p-4">
      <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex flex-wrap items-center gap-2 mb-1">
            <span className="text-sm text-muted-foreground">
              #{task.id}
            </span>
            <Badge
              variant="secondary"
              className={`${stateColors[task.state] || "bg-gray-500"} text-white text-xs`}
            >
              {stateLabels[task.state] || task.state}
            </Badge>
            {task.workspaceName && (
              <Badge variant="outline" className="text-xs">
                {task.workspaceName}
              </Badge>
            )}
            <PluginTaskBadges taskId={task.id} workspaceId={task.workspaceId} />
          </div>
          <h3 className="font-medium text-sm md:text-base line-clamp-2">
            {task.title}
          </h3>
          {task.description && (
            <p className="text-xs md:text-sm text-muted-foreground mt-1 line-clamp-2">
              {task.description}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2 self-end sm:self-start">
          {task.state === "new" && (
            <Button size="sm" onClick={() => onAssign(task.id)}>
              Assign
            </Button>
          )}
          {task.state === "queued" && (
            <Button size="sm" onClick={() => onStartTask(task.id)}>
              <Play className="h-4 w-4 mr-1" />
              Start
            </Button>
          )}
          {task.state === "needs_help" && onRetry && (
            <Button size="sm" variant="outline" onClick={() => onRetry(task.id)}>
              <RotateCcw className="h-4 w-4 mr-1" />
              Retry
            </Button>
          )}
          <PluginTaskActions taskId={task.id} workspaceId={task.workspaceId} />
        </div>
      </div>
    </Card>
  );
}
