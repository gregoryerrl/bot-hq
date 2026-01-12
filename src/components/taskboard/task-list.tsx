"use client";

import { useState, useEffect, useCallback } from "react";
import { TaskCard } from "./task-card";
import { Task } from "@/lib/db/schema";

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

  async function handleStartAgent(taskId: number) {
    try {
      await fetch("/api/agents/start", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ taskId }),
      });
      fetchTasks();
    } catch (error) {
      console.error("Failed to start agent:", error);
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
          onStartAgent={handleStartAgent}
        />
      ))}
    </div>
  );
}
