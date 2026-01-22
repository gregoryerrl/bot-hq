"use client";

import { useState, useEffect, useCallback } from "react";
import { TaskCard } from "./task-card";
import { Task } from "@/lib/db/schema";
import { toast } from "sonner";

interface TaskListProps {
  workspaceFilter?: number;
  stateFilter?: string;
}

export function TaskList({ workspaceFilter, stateFilter }: TaskListProps) {
  const [tasks, setTasks] = useState<(Task & { workspaceName?: string })[]>([]);
  const [loading, setLoading] = useState(true);

  const fetchTasks = useCallback(async () => {
    try {
      const params = new URLSearchParams();
      if (workspaceFilter) params.set("workspaceId", workspaceFilter.toString());
      if (stateFilter) params.set("state", stateFilter);

      const res = await fetch(`/api/tasks?${params}`);
      if (!res.ok) {
        console.error("Failed to fetch tasks:", res.status, res.statusText);
        setTasks([]);
        return;
      }

      const data = await res.json();

      // Ensure data is an array
      if (Array.isArray(data)) {
        setTasks(data);
      } else {
        console.error("API returned non-array:", data);
        setTasks([]);
      }
    } catch (error) {
      console.error("Failed to fetch tasks:", error);
      setTasks([]);
    } finally {
      setLoading(false);
    }
  }, [workspaceFilter, stateFilter]);

  useEffect(() => {
    fetchTasks();
    const interval = setInterval(fetchTasks, 5000);
    return () => clearInterval(interval);
  }, [fetchTasks]);

  async function handleAssign(taskId: number) {
    try {
      await fetch(`/api/tasks/${taskId}/assign`, { method: "POST" });
      fetchTasks();
    } catch (error) {
      console.error("Failed to assign task:", error);
    }
  }

  async function handleStartTask(taskId: number) {
    try {
      // First update task state to in_progress
      await fetch(`/api/tasks/${taskId}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ state: "in_progress" }),
      });

      // Send command to manager - simple and direct prompt
      const response = await fetch("/api/manager/command", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          command: `Work on bot-hq task #${taskId}. Fetch task details with task_get, then implement the requirements directly.`,
        }),
      });

      if (!response.ok) {
        throw new Error("Failed to send command to manager");
      }

      toast.success(`Task ${taskId} sent to manager`);
      fetchTasks();
    } catch (error) {
      console.error("Failed to start task:", error);
      toast.error("Failed to start task");
    }
  }

  async function handleRetry(taskId: number) {
    try {
      // Update task state back to queued
      await fetch(`/api/tasks/${taskId}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ state: "queued" }),
      });

      toast.success(`Task ${taskId} moved back to queue`);
      fetchTasks();
    } catch (error) {
      console.error("Failed to retry task:", error);
      toast.error("Failed to retry task");
    }
  }

  async function handleRetryWithFeedback(taskId: number, feedback: string) {
    try {
      // First get the current task to read its iterationCount
      const taskResponse = await fetch(`/api/tasks/${taskId}`);
      const task = await taskResponse.json();
      const currentIteration = task.iterationCount || 1;

      // Update task with feedback and increment iteration count
      await fetch(`/api/tasks/${taskId}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          state: "queued",
          feedback: feedback,
          iterationCount: currentIteration + 1,
        }),
      });

      toast.success(`Task ${taskId} requeued with feedback (iteration ${currentIteration + 1})`);
      fetchTasks();
    } catch (error) {
      console.error("Failed to retry task with feedback:", error);
      toast.error("Failed to retry task");
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading tasks...</div>;
  }

  if (tasks.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
        No tasks found. Click "Create Task" to add a new task.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {tasks.map((task) => (
        <TaskCard
          key={task.id}
          task={task}
          onAssign={handleAssign}
          onStartTask={handleStartTask}
          onRetry={handleRetry}
          onRetryWithFeedback={handleRetryWithFeedback}
        />
      ))}
    </div>
  );
}
