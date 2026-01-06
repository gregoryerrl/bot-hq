"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Play, ExternalLink } from "lucide-react";
import { Task } from "@/lib/db/schema";

interface TaskCardProps {
  task: Task & { workspaceName?: string };
  onAssign: (taskId: number) => void;
  onStartAgent: (taskId: number) => void;
}

const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  analyzing: "bg-blue-500",
  plan_ready: "bg-purple-500",
  in_progress: "bg-orange-500",
  pr_draft: "bg-green-500",
  done: "bg-green-700",
};

const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  analyzing: "Analyzing",
  plan_ready: "Plan Ready",
  in_progress: "In Progress",
  pr_draft: "PR Draft",
  done: "Done",
};

export function TaskCard({ task, onAssign, onStartAgent }: TaskCardProps) {
  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            {task.githubIssueNumber && (
              <span className="text-sm text-muted-foreground">
                #{task.githubIssueNumber}
              </span>
            )}
            <Badge
              variant="secondary"
              className={`${stateColors[task.state]} text-white`}
            >
              {stateLabels[task.state]}
            </Badge>
            {task.workspaceName && (
              <Badge variant="outline">{task.workspaceName}</Badge>
            )}
          </div>
          <h3 className="font-medium truncate">{task.title}</h3>
          {task.description && (
            <p className="text-sm text-muted-foreground mt-1 line-clamp-2">
              {task.description}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          {task.state === "new" && (
            <Button size="sm" onClick={() => onAssign(task.id)}>
              Assign
            </Button>
          )}
          {task.state === "queued" && (
            <Button size="sm" onClick={() => onStartAgent(task.id)}>
              <Play className="h-4 w-4 mr-1" />
              Start
            </Button>
          )}
          {task.prUrl && (
            <Button size="sm" variant="outline" asChild>
              <a href={task.prUrl} target="_blank" rel="noopener noreferrer">
                <ExternalLink className="h-4 w-4" />
              </a>
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}
