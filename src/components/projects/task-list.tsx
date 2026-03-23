"use client";

import { useEffect, useState, useCallback } from "react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { Plus } from "lucide-react";
import { toast } from "sonner";
import { TaskItem } from "./task-item";
import { AddTaskDialog } from "./add-task-dialog";

interface TaskData {
  id: number;
  title: string;
  description?: string | null;
  state: string;
  priority: number;
  tags?: string | null;
  dueDate?: string | Date | null;
  order: number;
  parentTaskId?: number | null;
}

interface TaskListProps {
  projectId: number;
}

export function TaskList({ projectId }: TaskListProps) {
  const [allTasks, setAllTasks] = useState<TaskData[]>([]);
  const [loading, setLoading] = useState(true);
  const [stateFilter, setStateFilter] = useState("all");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [parentTaskId, setParentTaskId] = useState<number | null>(null);

  const fetchTasks = useCallback(async () => {
    try {
      const res = await fetch(`/api/projects/${projectId}/tasks`);
      if (res.ok) {
        const data = await res.json();
        setAllTasks(data);
      }
    } finally {
      setLoading(false);
    }
  }, [projectId]);

  useEffect(() => {
    fetchTasks();
  }, [fetchTasks]);

  const handleStateChange = async (taskId: number, newState: string) => {
    try {
      const res = await fetch(`/api/tasks/${taskId}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ state: newState }),
      });
      if (!res.ok) {
        throw new Error("Failed to update task");
      }
      await fetchTasks();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to update task"
      );
    }
  };

  const handleDelete = async (taskId: number) => {
    try {
      const res = await fetch(`/api/tasks/${taskId}`, {
        method: "DELETE",
      });
      if (!res.ok) {
        throw new Error("Failed to delete task");
      }
      await fetchTasks();
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to delete task"
      );
    }
  };

  const handleAddSubtask = (parentId: number) => {
    setParentTaskId(parentId);
    setDialogOpen(true);
  };

  const handleAddTask = () => {
    setParentTaskId(null);
    setDialogOpen(true);
  };

  const handleCreated = () => {
    fetchTasks();
  };

  // Build tree: filter tasks, then organize into parent/child
  const filtered =
    stateFilter === "all"
      ? allTasks
      : allTasks.filter((t) => t.state === stateFilter);

  // Get IDs of filtered tasks for subtask lookup
  const filteredIds = new Set(filtered.map((t) => t.id));

  // Top-level tasks (no parent, or parent not in this project)
  const topLevel = filtered.filter(
    (t) => !t.parentTaskId || !filteredIds.has(t.parentTaskId)
  );

  // Build subtask map
  const subtaskMap = new Map<number, TaskData[]>();
  for (const task of filtered) {
    if (task.parentTaskId && filteredIds.has(task.parentTaskId)) {
      const existing = subtaskMap.get(task.parentTaskId) || [];
      existing.push(task);
      subtaskMap.set(task.parentTaskId, existing);
    }
  }

  return (
    <div>
      <div className="flex items-center justify-between gap-4 mb-4">
        <Select value={stateFilter} onValueChange={setStateFilter}>
          <SelectTrigger className="w-[160px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All</SelectItem>
            <SelectItem value="todo">Todo</SelectItem>
            <SelectItem value="in_progress">In Progress</SelectItem>
            <SelectItem value="done">Done</SelectItem>
            <SelectItem value="blocked">Blocked</SelectItem>
          </SelectContent>
        </Select>

        <Button size="sm" onClick={handleAddTask}>
          <Plus className="h-4 w-4" />
          Add Task
        </Button>
      </div>

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <p className="text-muted-foreground">Loading tasks...</p>
        </div>
      ) : topLevel.length === 0 ? (
        <div className="flex items-center justify-center py-12">
          <p className="text-muted-foreground">
            {stateFilter === "all"
              ? "No tasks yet. Add one to get started."
              : "No tasks match the current filter."}
          </p>
        </div>
      ) : (
        <div className="space-y-1">
          {topLevel.map((task) => (
            <TaskItem
              key={task.id}
              task={task}
              onStateChange={handleStateChange}
              onDelete={handleDelete}
              onAddSubtask={handleAddSubtask}
              subtasks={subtaskMap.get(task.id)}
            />
          ))}
        </div>
      )}

      <AddTaskDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        projectId={projectId}
        parentTaskId={parentTaskId}
        onCreated={handleCreated}
      />
    </div>
  );
}
