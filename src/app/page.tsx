"use client";

import { useState, useEffect, useCallback } from "react";
import { Header } from "@/components/layout/header";
import { SystemStatus } from "@/components/dashboard/system-status";
import { StatCard } from "@/components/dashboard/stat-card";
import { ActiveTasks } from "@/components/dashboard/active-tasks";
import { PendingReviews } from "@/components/dashboard/pending-reviews";
import { RecentActivity } from "@/components/dashboard/recent-activity";
import { Task } from "@/lib/db/schema";
import {
  ListOrdered,
  Loader2,
  AlertTriangle,
  GitPullRequest,
} from "lucide-react";

interface ManagerStatus {
  running: boolean;
  sessionId: string | null;
}

interface LogEntry {
  id: number;
  type: string;
  message: string;
  createdAt: string;
  workspaceName?: string;
  taskTitle?: string;
}

type TaskWithWorkspace = Task & { workspaceName?: string };

export default function DashboardPage() {
  const [managerStatus, setManagerStatus] = useState<ManagerStatus>({
    running: false,
    sessionId: null,
  });
  const [allTasks, setAllTasks] = useState<TaskWithWorkspace[]>([]);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [statusRes, tasksRes, logsRes] = await Promise.all([
        fetch("/api/manager/status"),
        fetch("/api/tasks"),
        fetch("/api/logs?limit=10"),
      ]);

      if (statusRes.ok) setManagerStatus(await statusRes.json());
      if (tasksRes.ok) setAllTasks(await tasksRes.json());
      if (logsRes.ok) setLogs(await logsRes.json());
    } catch (error) {
      console.error("Failed to fetch dashboard data:", error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  const queuedCount = allTasks.filter((t) => t.state === "queued").length;
  const inProgressCount = allTasks.filter((t) => t.state === "in_progress").length;
  const needsHelpCount = allTasks.filter((t) => t.state === "needs_help").length;
  const pendingReviewCount = allTasks.filter(
    (t) => t.state === "done" && t.branchName
  ).length;

  const activeTasks = allTasks
    .filter((t) => t.state === "in_progress" || t.state === "needs_help")
    .slice(0, 5);

  const pendingReviewTasks = allTasks
    .filter((t) => t.state === "done" && t.branchName)
    .slice(0, 5);

  return (
    <div className="flex flex-col h-full">
      <Header title="Dashboard" description="System overview and quick actions" />
      <div className="flex-1 p-4 md:p-6 space-y-6 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : (
          <>
            {/* System Status */}
            <SystemStatus status={managerStatus} />

            {/* Stat Cards */}
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
              <StatCard
                title="Queued"
                count={queuedCount}
                icon={ListOrdered}
                color="bg-yellow-500"
                href="/taskboard?state=queued"
              />
              <StatCard
                title="In Progress"
                count={inProgressCount}
                icon={Loader2}
                color="bg-orange-500"
                href="/taskboard?state=in_progress"
              />
              <StatCard
                title="Needs Help"
                count={needsHelpCount}
                icon={AlertTriangle}
                color="bg-red-500"
                href="/taskboard?state=needs_help"
              />
              <StatCard
                title="Pending Review"
                count={pendingReviewCount}
                icon={GitPullRequest}
                color="bg-blue-500"
                href="/pending"
              />
            </div>

            {/* Two-column layout */}
            <div className="grid grid-cols-1 md:grid-cols-5 gap-4">
              <div className="md:col-span-3">
                <ActiveTasks tasks={activeTasks} />
              </div>
              <div className="md:col-span-2">
                <PendingReviews tasks={pendingReviewTasks} />
              </div>
            </div>

            {/* Recent Activity */}
            <RecentActivity logs={logs} />
          </>
        )}
      </div>
    </div>
  );
}
