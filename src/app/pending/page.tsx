"use client";

import { useState, useEffect, useCallback } from "react";
import { Header } from "@/components/layout/header";
import { DiffReviewCard } from "@/components/review/diff-review-card";
import { WorkspaceSuggestionCard } from "@/components/review/workspace-suggestion-card";
import { CleanupSuggestionCard } from "@/components/review/cleanup-suggestion-card";
import { Task } from "@/lib/db/schema";
import { Badge } from "@/components/ui/badge";
import { toast } from "sonner";
import { ChevronDown, ChevronRight } from "lucide-react";

interface DiffFile {
  filename: string;
  additions: number;
  deletions: number;
}

interface ReviewData {
  task: Task & { workspaceName?: string };
  diff: {
    branch: string;
    baseBranch: string;
    files: DiffFile[];
    totalAdditions: number;
    totalDeletions: number;
  };
}

interface WorkspaceSuggestion {
  name: string;
  repoPath: string;
  remotes?: { gitName: string; provider: string; owner: string | null; repo: string | null }[];
}

interface CleanupSuggestion {
  name: string;
  path: string;
  reason: string;
  lastModified: string;
  isEmpty: boolean;
  hasGit: boolean;
}

export default function PendingPage() {
  const [reviews, setReviews] = useState<ReviewData[]>([]);
  const [loading, setLoading] = useState(true);

  // Discovery state
  const [workspaceSuggestions, setWorkspaceSuggestions] = useState<WorkspaceSuggestion[]>([]);
  const [cleanupSuggestions, setCleanupSuggestions] = useState<CleanupSuggestion[]>([]);
  const [dismissedWorkspaces, setDismissedWorkspaces] = useState<Set<string>>(new Set());
  const [dismissedCleanup, setDismissedCleanup] = useState<Set<string>>(new Set());
  const [workspacesExpanded, setWorkspacesExpanded] = useState(true);
  const [cleanupExpanded, setCleanupExpanded] = useState(true);

  const fetchReviews = useCallback(async () => {
    try {
      const res = await fetch("/api/tasks?state=done");
      if (!res.ok) {
        setReviews([]);
        return;
      }

      const allDoneTasks: (Task & { workspaceName?: string })[] = await res.json();
      const pendingTasks = allDoneTasks.filter((t) => t.branchName);

      const reviewPromises = pendingTasks.map(async (task) => {
        try {
          const reviewRes = await fetch(`/api/tasks/${task.id}/review`);
          if (!reviewRes.ok) return null;
          return await reviewRes.json() as ReviewData;
        } catch {
          return null;
        }
      });

      const reviewResults = await Promise.all(reviewPromises);
      setReviews(reviewResults.filter(Boolean) as ReviewData[]);
    } catch (error) {
      console.error("Failed to fetch reviews:", error);
      setReviews([]);
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchDiscovery = useCallback(async () => {
    try {
      const res = await fetch("/api/workspaces/discover");
      if (!res.ok) return;
      const data = await res.json();
      setWorkspaceSuggestions(data.workspaces || []);
      setCleanupSuggestions(data.cleanup || []);
    } catch (error) {
      console.error("Failed to fetch discovery:", error);
    }
  }, []);

  useEffect(() => {
    fetchReviews();
    fetchDiscovery();
    const interval = setInterval(fetchReviews, 10000);
    return () => clearInterval(interval);
  }, [fetchReviews, fetchDiscovery]);

  async function handleAccept(taskId: number) {
    try {
      const res = await fetch(`/api/tasks/${taskId}/review`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "accept" }),
      });

      if (!res.ok) throw new Error("Failed to accept");

      toast.success(`Task ${taskId} accepted and committed`);
      fetchReviews();
    } catch (error) {
      console.error("Failed to accept task:", error);
      toast.error("Failed to accept task");
    }
  }

  async function handleDelete(taskId: number) {
    try {
      const res = await fetch(`/api/tasks/${taskId}/review`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "delete" }),
      });

      if (!res.ok) throw new Error("Failed to delete");

      toast.success(`Task ${taskId} deleted`);
      fetchReviews();
    } catch (error) {
      console.error("Failed to delete task:", error);
      toast.error("Failed to delete task");
    }
  }

  async function handleRetry(taskId: number, feedback: string) {
    try {
      const res = await fetch(`/api/tasks/${taskId}/review`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "retry", feedback }),
      });

      if (!res.ok) throw new Error("Failed to retry");

      toast.success(`Task ${taskId} requeued with feedback`);
      fetchReviews();
    } catch (error) {
      console.error("Failed to retry task:", error);
      toast.error("Failed to retry task");
    }
  }

  const visibleWorkspaces = workspaceSuggestions.filter(
    (w) => !dismissedWorkspaces.has(w.repoPath)
  );
  const visibleCleanup = cleanupSuggestions.filter(
    (c) => !dismissedCleanup.has(c.path)
  );

  const hasAnySuggestions = visibleWorkspaces.length > 0 || visibleCleanup.length > 0;
  const hasAnyContent = reviews.length > 0 || hasAnySuggestions;

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Review"
        description="Review uncommitted changes from completed tasks"
      />
      <div className="flex-1 p-4 md:p-6 space-y-6">
        {/* Workspace Suggestions */}
        {visibleWorkspaces.length > 0 && (
          <div>
            <button
              onClick={() => setWorkspacesExpanded(!workspacesExpanded)}
              className="flex items-center gap-2 mb-3 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
            >
              {workspacesExpanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              )}
              Workspace Suggestions
              <Badge variant="secondary">{visibleWorkspaces.length}</Badge>
            </button>
            {workspacesExpanded && (
              <div className="space-y-2">
                {visibleWorkspaces.map((ws) => (
                  <WorkspaceSuggestionCard
                    key={ws.repoPath}
                    name={ws.name}
                    repoPath={ws.repoPath}
                    remotes={ws.remotes}
                    onAccepted={() => {
                      fetchDiscovery();
                    }}
                    onDismissed={() => {
                      setDismissedWorkspaces((prev) => new Set([...prev, ws.repoPath]));
                    }}
                  />
                ))}
              </div>
            )}
          </div>
        )}

        {/* Cleanup Suggestions */}
        {visibleCleanup.length > 0 && (
          <div>
            <button
              onClick={() => setCleanupExpanded(!cleanupExpanded)}
              className="flex items-center gap-2 mb-3 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
            >
              {cleanupExpanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              )}
              Cleanup Suggestions
              <Badge variant="secondary">{visibleCleanup.length}</Badge>
            </button>
            {cleanupExpanded && (
              <div className="space-y-2">
                {visibleCleanup.map((item) => (
                  <CleanupSuggestionCard
                    key={item.path}
                    name={item.name}
                    path={item.path}
                    reason={item.reason}
                    onAccepted={() => {
                      fetchDiscovery();
                    }}
                    onDismissed={() => {
                      setDismissedCleanup((prev) => new Set([...prev, item.path]));
                    }}
                  />
                ))}
              </div>
            )}
          </div>
        )}

        {/* Task Reviews */}
        {loading ? (
          <div className="text-muted-foreground">Loading reviews...</div>
        ) : reviews.length === 0 && !hasAnySuggestions ? (
          <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
            No tasks pending review. Completed tasks with uncommitted changes will appear here.
          </div>
        ) : reviews.length > 0 ? (
          <div className="space-y-4">
            {reviews.map((review) => (
              <DiffReviewCard
                key={review.task.id}
                task={review.task}
                diff={review.diff}
                onAccept={handleAccept}
                onDelete={handleDelete}
                onRetry={handleRetry}
              />
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}
