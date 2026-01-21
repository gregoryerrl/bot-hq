"use client";

import { useState } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Play, RotateCcw, MessageSquare, X, ExternalLink } from "lucide-react";
import { Task } from "@/lib/db/schema";

interface TaskCardProps {
  task: Task & { workspaceName?: string };
  onAssign: (taskId: number) => void;
  onStartTask: (taskId: number) => void;
  onRetry?: (taskId: number) => void;
  onRetryWithFeedback?: (taskId: number, feedback: string) => void;
}

const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  in_progress: "bg-orange-500",
  awaiting_input: "bg-amber-500",
  needs_help: "bg-red-500",
  done: "bg-green-700",
};

const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  in_progress: "In Progress",
  awaiting_input: "Awaiting Input",
  needs_help: "Needs Help",
  done: "Done",
};

export function TaskCard({ task, onAssign, onStartTask, onRetry, onRetryWithFeedback }: TaskCardProps) {
  const [showFeedback, setShowFeedback] = useState(false);
  const [feedback, setFeedback] = useState("");

  const handleRetryWithFeedback = () => {
    if (onRetryWithFeedback && feedback.trim()) {
      onRetryWithFeedback(task.id, feedback);
      setFeedback("");
      setShowFeedback(false);
    }
  };

  // Check if task is from a git remote (has sourceRef)
  const hasSourceRef = task.sourceRef && task.sourceRemoteId;

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
              className={`${stateColors[task.state] || "bg-gray-500"} text-white text-xs ${
                task.state === "awaiting_input" ? "animate-pulse" : ""
              }`}
            >
              {stateLabels[task.state] || task.state}
            </Badge>
            {task.workspaceName && (
              <Badge variant="outline" className="text-xs">
                {task.workspaceName}
              </Badge>
            )}
            {hasSourceRef && (
              <Badge variant="outline" className="text-xs text-blue-600 border-blue-600">
                Issue #{task.sourceRef}
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
          {task.state === "done" && onRetryWithFeedback && (
            <Button size="sm" variant="outline" onClick={() => setShowFeedback(!showFeedback)}>
              <MessageSquare className="h-4 w-4 mr-1" />
              Request Changes
            </Button>
          )}
        </div>
      </div>

      {/* Feedback input for done tasks */}
      {showFeedback && task.state === "done" && (
        <div className="mt-3 pt-3 border-t space-y-2">
          <Textarea
            placeholder="Describe what changes you need..."
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            className="min-h-[80px] text-sm"
          />
          <div className="flex gap-2">
            <Button size="sm" onClick={handleRetryWithFeedback} disabled={!feedback.trim()}>
              <RotateCcw className="h-4 w-4 mr-1" />
              Submit & Retry
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setShowFeedback(false)}>
              <X className="h-4 w-4 mr-1" />
              Cancel
            </Button>
          </div>
        </div>
      )}
    </Card>
  );
}
