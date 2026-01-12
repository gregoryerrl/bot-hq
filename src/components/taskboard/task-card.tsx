"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Play } from "lucide-react";
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
  pending_review: "bg-green-500",
  done: "bg-green-700",
};

const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  analyzing: "Analyzing",
  plan_ready: "Plan Ready",
  in_progress: "In Progress",
  pending_review: "Pending Review",
  done: "Done",
};

export function TaskCard({ task, onAssign, onStartAgent }: TaskCardProps) {
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
              className={`${stateColors[task.state]} text-white text-xs`}
            >
              {stateLabels[task.state]}
            </Badge>
            {task.workspaceName && (
              <Badge variant="outline" className="text-xs">
                {task.workspaceName}
              </Badge>
            )}
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
            <Button size="sm" onClick={() => onStartAgent(task.id)}>
              <Play className="h-4 w-4 mr-1" />
              Start
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}
