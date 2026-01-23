"use client";

import { useState, useEffect, useCallback } from "react";
import { AlertCircle, ChevronDown, ChevronUp, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface AwaitingTask {
  id: number;
  title: string;
  workspaceName: string;
  waitingQuestion: string | null;
  waitingContext: string | null;
  waitingSince: string | null;
}

export function AwaitingInputBanner() {
  const [tasks, setTasks] = useState<AwaitingTask[]>([]);
  const [expanded, setExpanded] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  const fetchAwaitingTasks = useCallback(async () => {
    try {
      const res = await fetch("/api/tasks/awaiting");
      if (res.ok) {
        const data = await res.json();
        setTasks(data);
        // If new tasks appear after dismissal, show again
        if (data.length > 0 && dismissed) {
          setDismissed(false);
        }
      }
    } catch (error) {
      console.error("Failed to fetch awaiting tasks:", error);
    }
  }, [dismissed]);

  useEffect(() => {
    fetchAwaitingTasks();
    const interval = setInterval(fetchAwaitingTasks, 5000);
    return () => clearInterval(interval);
  }, [fetchAwaitingTasks]);

  // Don't show if no tasks or dismissed
  if (tasks.length === 0 || dismissed) {
    return null;
  }

  const firstTask = tasks[0];
  const hasMultiple = tasks.length > 1;

  return (
    <div className="bg-amber-500/10 border-b border-amber-500/30 text-amber-200">
      <div className="max-w-7xl mx-auto px-4 py-2">
        {/* Main banner row */}
        <div className="flex items-center gap-3">
          <div className="relative">
            <AlertCircle className="h-5 w-5 text-amber-500 animate-pulse" />
          </div>

          <div className="flex-1 min-w-0">
            {hasMultiple ? (
              <button
                onClick={() => setExpanded(!expanded)}
                className="flex items-center gap-2 text-sm font-medium text-amber-100 hover:text-white"
              >
                <span>{tasks.length} tasks awaiting your input</span>
                {expanded ? (
                  <ChevronUp className="h-4 w-4" />
                ) : (
                  <ChevronDown className="h-4 w-4" />
                )}
              </button>
            ) : (
              <div className="text-sm">
                <span className="font-medium text-amber-100">
                  Manager needs input on{" "}
                  <a
                    href="/"
                    className="underline hover:text-white"
                  >
                    Task #{firstTask.id}
                  </a>
                </span>
                {firstTask.waitingQuestion && (
                  <span className="text-amber-200/80 ml-2 truncate">
                    &quot;{firstTask.waitingQuestion.slice(0, 60)}
                    {firstTask.waitingQuestion.length > 60 ? "..." : ""}&quot;
                  </span>
                )}
              </div>
            )}
          </div>

          <a href="/claude">
            <Button
              size="sm"
              variant="outline"
              className="border-amber-500/50 text-amber-100 hover:bg-amber-500/20 hover:text-white"
            >
              Go to Terminal
            </Button>
          </a>

          <Button
            size="icon"
            variant="ghost"
            className="h-8 w-8 text-amber-300 hover:text-white hover:bg-amber-500/20"
            onClick={() => setDismissed(true)}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* Expanded list for multiple tasks */}
        {expanded && hasMultiple && (
          <div className="mt-3 space-y-2 border-t border-amber-500/20 pt-3">
            {tasks.map((task) => (
              <div
                key={task.id}
                className={cn(
                  "flex items-start gap-3 p-2 rounded-md",
                  "bg-amber-500/10 hover:bg-amber-500/20 transition-colors"
                )}
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-amber-400">
                      #{task.id}
                    </span>
                    <span className="text-sm font-medium text-amber-100 truncate">
                      {task.title}
                    </span>
                    <span className="text-xs text-amber-400/60">
                      ({task.workspaceName})
                    </span>
                  </div>
                  {task.waitingQuestion && (
                    <p className="text-xs text-amber-200/70 mt-1 truncate">
                      {task.waitingQuestion}
                    </p>
                  )}
                  {task.waitingContext && (
                    <pre className="text-xs text-amber-200/60 mt-1 whitespace-pre-wrap">
                      {task.waitingContext}
                    </pre>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Single task expanded details */}
        {expanded && !hasMultiple && firstTask.waitingContext && (
          <div className="mt-3 border-t border-amber-500/20 pt-3">
            <p className="text-sm text-amber-100 mb-2">
              {firstTask.waitingQuestion}
            </p>
            <pre className="text-xs text-amber-200/70 whitespace-pre-wrap bg-amber-500/10 rounded p-2">
              {firstTask.waitingContext}
            </pre>
          </div>
        )}
      </div>
    </div>
  );
}
