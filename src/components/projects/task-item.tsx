"use client";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import {
  Circle,
  Loader2,
  CheckCircle2,
  XCircle,
  MoreHorizontal,
  Plus,
  Trash2,
} from "lucide-react";
import { format, isPast, isToday } from "date-fns";

interface TaskItemTask {
  id: number;
  title: string;
  description?: string | null;
  state: string;
  priority: number;
  tags?: string | null;
  dueDate?: string | Date | null;
  order: number;
}

interface TaskItemProps {
  task: TaskItemTask;
  onStateChange: (taskId: number, newState: string) => void;
  onDelete: (taskId: number) => void;
  onAddSubtask: (parentTaskId: number) => void;
  subtasks?: TaskItemTask[];
  depth?: number;
}

const stateOrder = ["todo", "in_progress", "done", "blocked"] as const;

function getNextState(current: string): string {
  if (current === "todo") return "in_progress";
  if (current === "in_progress") return "done";
  if (current === "done") return "todo";
  return "todo"; // blocked -> todo
}

function StateIcon({ state }: { state: string }) {
  switch (state) {
    case "in_progress":
      return <Loader2 className="h-4 w-4 text-yellow-500 animate-spin" />;
    case "done":
      return <CheckCircle2 className="h-4 w-4 text-green-500" />;
    case "blocked":
      return <XCircle className="h-4 w-4 text-red-500" />;
    default:
      return <Circle className="h-4 w-4 text-muted-foreground" />;
  }
}

function PriorityDot({ priority }: { priority: number }) {
  if (priority <= 0) return null;
  const color =
    priority === 1
      ? "bg-blue-500"
      : priority === 2
        ? "bg-orange-500"
        : "bg-red-500";
  return <span className={`inline-block h-2 w-2 rounded-full ${color}`} />;
}

function parseTags(tags: string | null | undefined): string[] {
  if (!tags) return [];
  try {
    const parsed = JSON.parse(tags);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function formatDueDate(dueDate: string | Date | null | undefined): {
  text: string;
  overdue: boolean;
} | null {
  if (!dueDate) return null;
  const date = new Date(dueDate);
  if (isNaN(date.getTime())) return null;
  const overdue = isPast(date) && !isToday(date);
  return { text: format(date, "MMM d, yyyy"), overdue };
}

export function TaskItem({
  task,
  onStateChange,
  onDelete,
  onAddSubtask,
  subtasks = [],
  depth = 0,
}: TaskItemProps) {
  const tags = parseTags(task.tags);
  const due = formatDueDate(task.dueDate);

  return (
    <div>
      <div
        className="flex items-center gap-2 py-2 px-2 rounded-md hover:bg-muted/50 group"
        style={{ marginLeft: `${depth * 1.5}rem` }}
      >
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6 shrink-0"
          onClick={() => onStateChange(task.id, getNextState(task.state))}
        >
          <StateIcon state={task.state} />
        </Button>

        <span
          className={`flex-1 text-sm truncate ${task.state === "done" ? "line-through text-muted-foreground" : ""}`}
        >
          {task.title}
        </span>

        <div className="flex items-center gap-2 shrink-0">
          <PriorityDot priority={task.priority} />

          {tags.map((tag) => (
            <Badge key={tag} variant="secondary" className="text-xs">
              {tag}
            </Badge>
          ))}

          {due && (
            <span
              className={`text-xs ${due.overdue ? "text-red-500 font-medium" : "text-muted-foreground"}`}
            >
              {due.text}
            </span>
          )}

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 opacity-0 group-hover:opacity-100 transition-opacity"
              >
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => onAddSubtask(task.id)}>
                <Plus className="h-4 w-4" />
                Add Subtask
              </DropdownMenuItem>
              <DropdownMenuItem
                variant="destructive"
                onClick={() => onDelete(task.id)}
              >
                <Trash2 className="h-4 w-4" />
                Delete
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {subtasks.length > 0 &&
        subtasks.map((sub) => (
          <TaskItem
            key={sub.id}
            task={sub}
            onStateChange={onStateChange}
            onDelete={onDelete}
            onAddSubtask={onAddSubtask}
            depth={depth + 1}
          />
        ))}
    </div>
  );
}
