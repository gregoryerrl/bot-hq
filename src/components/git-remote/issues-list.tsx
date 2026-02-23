"use client";

import { useState, useEffect, useCallback } from "react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  RefreshCw, Plus, ExternalLink, CheckCircle2, Loader2,
  CircleDot, FolderGit2, KeyRound, MessageSquare, User, ChevronDown, ChevronRight,
} from "lucide-react";
import { toast } from "sonner";

interface GitHubIssue {
  number: number;
  title: string;
  body: string | null;
  state: string;
  labels: string[];
  url: string;
  author: string;
  createdAt: string;
  commentsCount: number;
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

interface SkippedWorkspace {
  workspaceId: number;
  workspaceName: string;
  owner: string;
  repo: string;
  reason: string;
}

interface IssuesResponse {
  workspaces: WorkspaceIssues[];
  skippedWorkspaces?: SkippedWorkspace[];
  hasGlobalToken?: boolean;
  totalIssues: number;
  issuesWithTasks: number;
}

interface IssuesListProps {
  workspaceId?: string;
}

function timeAgo(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);

  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.floor(months / 12)}y ago`;
}

export function IssuesList({ workspaceId }: IssuesListProps) {
  const [issuesData, setIssuesData] = useState<IssuesResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [creatingTask, setCreatingTask] = useState<string | null>(null);
  const [expandedIssue, setExpandedIssue] = useState<string | null>(null);

  const wsId = workspaceId ? Number(workspaceId) : null;

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

  async function handleCreateTask(targetWorkspaceId: number, issueNumber: number) {
    const key = `${targetWorkspaceId}-${issueNumber}`;
    setCreatingTask(key);
    try {
      const res = await fetch("/api/git-remote/issues/create-task", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ workspaceId: targetWorkspaceId, issueNumber }),
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

  function toggleExpand(key: string) {
    setExpandedIssue((prev) => (prev === key ? null : key));
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

  const filteredWorkspaces = wsId
    ? (issuesData?.workspaces || []).filter((w) => w.workspaceId === wsId)
    : issuesData?.workspaces || [];

  const filteredSkipped = wsId
    ? (issuesData?.skippedWorkspaces || []).filter((w) => w.workspaceId === wsId)
    : issuesData?.skippedWorkspaces || [];

  const totalFilteredIssues = filteredWorkspaces.reduce((sum, w) => sum + w.issues.length, 0);
  const hasSkippedNoToken = filteredSkipped.some((s) => s.reason === "no_token");

  if (filteredWorkspaces.length === 0) {
    if (hasSkippedNoToken) {
      const skipped = filteredSkipped.filter((s) => s.reason === "no_token");
      return (
        <Card>
          <CardContent className="py-12">
            <div className="text-center text-muted-foreground">
              <KeyRound className="h-12 w-12 mx-auto mb-4 opacity-50" />
              <p className="font-medium">GitHub Token Required</p>
              <p className="text-sm mt-2 max-w-md mx-auto">
                {wsId
                  ? `Remote ${skipped[0]?.owner}/${skipped[0]?.repo} was detected but has no token. Add a GitHub token to a global remote or this workspace's remote to fetch issues.`
                  : `${skipped.length} workspace${skipped.length > 1 ? "s have" : " has"} remotes detected but no GitHub token configured. Add a global GitHub remote with a token under Remotes \u2192 Add Remote.`}
              </p>
              <div className="flex flex-wrap justify-center gap-2 mt-4">
                {skipped.map((s) => (
                  <Badge key={s.workspaceId} variant="outline" className="text-xs">
                    {s.owner}/{s.repo}
                  </Badge>
                ))}
              </div>
            </div>
          </CardContent>
        </Card>
      );
    }

    return (
      <Card>
        <CardContent className="py-12">
          <div className="text-center text-muted-foreground">
            <CircleDot className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p className="font-medium">No Issues Found</p>
            <p className="text-sm mt-2">
              {wsId
                ? "No remote configured for this workspace. Go to Remotes tab to detect or add one."
                : "Configure a Git Remote with owner/repo and a GitHub token to view issues."}
            </p>
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>Open Issues</CardTitle>
              <CardDescription>
                {totalFilteredIssues} open issue{totalFilteredIssues !== 1 ? "s" : ""}
                {!wsId && filteredWorkspaces.length > 1 && (
                  <span> across {filteredWorkspaces.length} workspaces</span>
                )}
                {filteredWorkspaces.reduce((sum, w) => sum + w.issues.filter((i) => i.hasTask).length, 0) > 0 && (
                  <span className="ml-1">
                    ({filteredWorkspaces.reduce((sum, w) => sum + w.issues.filter((i) => i.hasTask).length, 0)} already have tasks)
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
            {filteredWorkspaces.map((workspace) => (
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
                    {workspace.issues.map((issue) => {
                      const issueKey = `${workspace.workspaceId}-${issue.number}`;
                      const isExpanded = expandedIssue === issueKey;

                      return (
                        <div
                          key={issue.number}
                          className="rounded-lg bg-muted/30 border overflow-hidden"
                        >
                          {/* Issue header row */}
                          <div className="flex items-start justify-between gap-4 p-3">
                            <div className="flex-1 min-w-0">
                              <div className="flex items-center gap-2 mb-1">
                                <button
                                  onClick={() => toggleExpand(issueKey)}
                                  className="text-muted-foreground hover:text-foreground shrink-0"
                                >
                                  {isExpanded ? (
                                    <ChevronDown className="h-4 w-4" />
                                  ) : (
                                    <ChevronRight className="h-4 w-4" />
                                  )}
                                </button>
                                <span className="text-xs text-muted-foreground font-mono">
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
                              {/* Meta row: author, time, comments, labels */}
                              <div className="flex items-center gap-3 ml-6 text-xs text-muted-foreground flex-wrap">
                                {issue.author && (
                                  <span className="flex items-center gap-1">
                                    <User className="h-3 w-3" />
                                    {issue.author}
                                  </span>
                                )}
                                {issue.createdAt && (
                                  <span title={new Date(issue.createdAt).toLocaleString()}>
                                    {timeAgo(issue.createdAt)}
                                  </span>
                                )}
                                {issue.commentsCount > 0 && (
                                  <span className="flex items-center gap-1">
                                    <MessageSquare className="h-3 w-3" />
                                    {issue.commentsCount}
                                  </span>
                                )}
                                {issue.labels.length > 0 && (
                                  <div className="flex flex-wrap gap-1">
                                    {issue.labels.map((label) => (
                                      <Badge
                                        key={label}
                                        variant="secondary"
                                        className="text-xs py-0"
                                      >
                                        {label}
                                      </Badge>
                                    ))}
                                  </div>
                                )}
                              </div>
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
                                  disabled={creatingTask === issueKey}
                                >
                                  {creatingTask === issueKey ? (
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

                          {/* Expanded body */}
                          {isExpanded && issue.body && (
                            <div className="px-3 pb-3 ml-6">
                              <div className="border-t pt-3 mt-1">
                                <div className="prose prose-sm dark:prose-invert max-w-none text-sm text-muted-foreground whitespace-pre-wrap break-words">
                                  {issue.body.length > 1000
                                    ? issue.body.slice(0, 1000) + "..."
                                    : issue.body}
                                </div>
                                <a
                                  href={issue.url}
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="text-xs text-blue-600 hover:underline mt-2 inline-flex items-center gap-1"
                                >
                                  View full issue on GitHub
                                  <ExternalLink className="h-3 w-3" />
                                </a>
                              </div>
                            </div>
                          )}
                          {isExpanded && !issue.body && (
                            <div className="px-3 pb-3 ml-6">
                              <div className="border-t pt-3 mt-1">
                                <p className="text-sm text-muted-foreground italic">No description provided.</p>
                              </div>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            ))}
          </div>
        </CardContent>
      </Card>

      {filteredSkipped.filter((s) => s.reason === "no_token").length > 0 && (
        <Card className="border-yellow-200">
          <CardContent className="py-4">
            <div className="flex items-start gap-3">
              <KeyRound className="h-5 w-5 text-yellow-600 shrink-0 mt-0.5" />
              <div className="text-sm">
                <p className="font-medium text-yellow-700">
                  {filteredSkipped.filter((s) => s.reason === "no_token").length} workspace{filteredSkipped.filter((s) => s.reason === "no_token").length > 1 ? "s" : ""} skipped — no GitHub token
                </p>
                <div className="flex flex-wrap gap-1 mt-1">
                  {filteredSkipped
                    .filter((s) => s.reason === "no_token")
                    .map((s) => (
                      <Badge key={s.workspaceId} variant="outline" className="text-xs">
                        {s.owner}/{s.repo}
                      </Badge>
                    ))}
                </div>
              </div>
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
