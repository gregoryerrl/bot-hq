"use client";

import { useState, useEffect, useCallback } from "react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { RefreshCw, Plus, ExternalLink, CheckCircle2, Loader2, CircleDot, FolderGit2 } from "lucide-react";
import { toast } from "sonner";

interface GitHubIssue {
  number: number;
  title: string;
  body: string | null;
  state: string;
  labels: string[];
  url: string;
  hasTask: boolean;
  taskId?: number;
}

interface WorkspaceIssues {
  workspaceId: number;
  workspaceName: string;
  owner: string;
  repo: string;
  issues: GitHubIssue[];
}

interface IssuesResponse {
  workspaces: WorkspaceIssues[];
  totalIssues: number;
  issuesWithTasks: number;
}

export function IssuesList() {
  const [issuesData, setIssuesData] = useState<IssuesResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [creatingTask, setCreatingTask] = useState<string | null>(null);

  const fetchIssues = useCallback(async () => {
    try {
      setLoading(true);
      const res = await fetch("/api/git-remote/issues");
      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Failed to fetch issues");
      }
      const result = await res.json();
      setIssuesData(result);
    } catch (error) {
      console.error("Failed to fetch issues:", error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchIssues();
  }, [fetchIssues]);

  async function handleCreateTask(workspaceId: number, issueNumber: number) {
    const key = `${workspaceId}-${issueNumber}`;
    setCreatingTask(key);
    try {
      const res = await fetch("/api/git-remote/issues/create-task", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ workspaceId, issueNumber }),
      });

      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Failed to create task");
      }

      const result = await res.json();
      toast.success(`Task created: ${result.title}`);
      fetchIssues();
    } catch (error) {
      console.error("Failed to create task:", error);
      toast.error(error instanceof Error ? error.message : "Failed to create task");
    } finally {
      setCreatingTask(null);
    }
  }

  if (loading && !issuesData) {
    return (
      <Card>
        <CardContent className="py-12">
          <div className="flex items-center justify-center text-muted-foreground">
            <Loader2 className="h-6 w-6 animate-spin mr-2" />
            Loading issues...
          </div>
        </CardContent>
      </Card>
    );
  }

  if (!issuesData || issuesData.workspaces.length === 0) {
    return (
      <Card>
        <CardContent className="py-12">
          <div className="text-center text-muted-foreground">
            <CircleDot className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p className="font-medium">No Issues Found</p>
            <p className="text-sm mt-2">
              Configure a Git Remote with owner/repo to view issues.
            </p>
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-6">
      {/* Connected Workspaces Summary */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>Open Issues</CardTitle>
              <CardDescription>
                {issuesData.totalIssues} open issues across {issuesData.workspaces.length} workspaces
                {issuesData.issuesWithTasks > 0 && (
                  <span className="ml-1">
                    ({issuesData.issuesWithTasks} already have tasks)
                  </span>
                )}
              </CardDescription>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={fetchIssues}
              disabled={loading}
            >
              <RefreshCw className={`h-4 w-4 mr-1 ${loading ? "animate-spin" : ""}`} />
              Refresh
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          <div className="space-y-6">
            {issuesData.workspaces.map((workspace) => (
              <div key={workspace.workspaceId}>
                <div className="flex items-center justify-between mb-3">
                  <div className="flex items-center gap-2">
                    <FolderGit2 className="h-4 w-4 text-muted-foreground" />
                    <span className="font-medium">{workspace.workspaceName}</span>
                  </div>
                  <a
                    href={`https://github.com/${workspace.owner}/${workspace.repo}/issues`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-sm text-muted-foreground hover:text-foreground flex items-center gap-1"
                  >
                    {workspace.owner}/{workspace.repo}
                    <ExternalLink className="h-3 w-3" />
                  </a>
                </div>
                {workspace.issues.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No open issues</p>
                ) : (
                  <div className="space-y-2">
                    {workspace.issues.map((issue) => (
                      <div
                        key={issue.number}
                        className="flex items-start justify-between gap-4 p-3 rounded-lg bg-muted/30 border"
                      >
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2 mb-1">
                            <span className="text-xs text-muted-foreground">
                              #{issue.number}
                            </span>
                            <a
                              href={issue.url}
                              target="_blank"
                              rel="noopener noreferrer"
                              className="font-medium hover:underline truncate"
                            >
                              {issue.title}
                            </a>
                          </div>
                          {issue.labels.length > 0 && (
                            <div className="flex flex-wrap gap-1 mt-1">
                              {issue.labels.map((label) => (
                                <Badge
                                  key={label}
                                  variant="secondary"
                                  className="text-xs"
                                >
                                  {label}
                                </Badge>
                              ))}
                            </div>
                          )}
                        </div>
                        <div className="flex items-center gap-2 shrink-0">
                          {issue.hasTask ? (
                            <Badge
                              variant="outline"
                              className="text-green-600 border-green-600"
                            >
                              <CheckCircle2 className="h-3 w-3 mr-1" />
                              Task #{issue.taskId}
                            </Badge>
                          ) : (
                            <Button
                              size="sm"
                              variant="outline"
                              onClick={() =>
                                handleCreateTask(workspace.workspaceId, issue.number)
                              }
                              disabled={
                                creatingTask ===
                                `${workspace.workspaceId}-${issue.number}`
                              }
                            >
                              {creatingTask ===
                              `${workspace.workspaceId}-${issue.number}` ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <>
                                  <Plus className="h-4 w-4 mr-1" />
                                  Create Task
                                </>
                              )}
                            </Button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
