"use client";

import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  RefreshCw,
  Plus,
  ExternalLink,
  CheckCircle2,
  Loader2,
  FolderGit2,
  GitBranch,
  AlertCircle,
  Check,
  Eye,
  EyeOff,
  CircleDot,
} from "lucide-react";
import { toast } from "sonner";

interface GitHubIssue {
  number: number;
  title: string;
  body: string | null;
  state: string;
  labels: string[];
  assignees: string[];
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

interface GitHubRepo {
  owner: string;
  name: string;
  fullName: string;
  description: string | null;
  private: boolean;
  url: string;
}

export function GitHubPluginPage() {
  // Settings state
  const [token, setToken] = useState("");
  const [showToken, setShowToken] = useState(false);
  const [savingToken, setSavingToken] = useState(false);
  const [tokenSaved, setTokenSaved] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<"unknown" | "connected" | "error">("unknown");

  // Repos state
  const [repos, setRepos] = useState<GitHubRepo[]>([]);
  const [loadingRepos, setLoadingRepos] = useState(false);
  const [selectedRepo, setSelectedRepo] = useState<string>("");
  const [clonePath, setClonePath] = useState("");
  const [cloning, setCloning] = useState(false);
  const [scopePath, setScopePath] = useState("");

  // Issues state
  const [issuesData, setIssuesData] = useState<IssuesResponse | null>(null);
  const [loadingIssues, setLoadingIssues] = useState(true);
  const [creatingTask, setCreatingTask] = useState<string | null>(null);

  // Load initial data
  useEffect(() => {
    loadCredentials();
    loadScopePath();
    fetchIssues();
  }, []);

  async function loadCredentials() {
    try {
      const res = await fetch("/api/plugins/github/credentials");
      if (res.ok) {
        const data = await res.json();
        if (data.hasToken) {
          setTokenSaved(true);
          setConnectionStatus(data.connected ? "connected" : "error");
          if (data.connected) {
            fetchRepos();
          }
        }
      }
    } catch (error) {
      console.error("Failed to load credentials:", error);
    }
  }

  async function loadScopePath() {
    try {
      const res = await fetch("/api/settings");
      if (res.ok) {
        const data = await res.json();
        setScopePath(data.scopePath || "");
      }
    } catch (error) {
      console.error("Failed to load scope path:", error);
    }
  }

  async function handleSaveToken() {
    if (!token.trim()) {
      toast.error("Please enter a GitHub token");
      return;
    }

    setSavingToken(true);
    try {
      const res = await fetch("/api/plugins/github/credentials", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ token }),
      });

      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Failed to save token");
      }

      const data = await res.json();
      setTokenSaved(true);
      setToken("");
      setConnectionStatus(data.connected ? "connected" : "error");
      toast.success("GitHub token saved successfully");

      if (data.connected) {
        fetchRepos();
      }
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Failed to save token");
      setConnectionStatus("error");
    } finally {
      setSavingToken(false);
    }
  }

  async function fetchRepos() {
    setLoadingRepos(true);
    try {
      const res = await fetch("/api/plugins/github/repos");
      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Failed to fetch repos");
      }
      const data = await res.json();
      setRepos(data.repos || []);
    } catch (error) {
      console.error("Failed to fetch repos:", error);
      toast.error(error instanceof Error ? error.message : "Failed to fetch repos");
    } finally {
      setLoadingRepos(false);
    }
  }

  // Auto-set clone path when repo is selected
  useEffect(() => {
    if (selectedRepo && scopePath) {
      const repo = repos.find(r => r.fullName === selectedRepo);
      if (repo) {
        setClonePath(`${scopePath}/${repo.name}`);
      }
    }
  }, [selectedRepo, scopePath, repos]);

  async function handleClone() {
    if (!selectedRepo) {
      toast.error("Please select a repository");
      return;
    }

    const repo = repos.find(r => r.fullName === selectedRepo);
    if (!repo) return;

    setCloning(true);
    try {
      const res = await fetch("/api/plugins/github/clone", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          owner: repo.owner,
          repo: repo.name,
          localPath: clonePath || undefined,
        }),
      });

      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Failed to clone repository");
      }

      const data = await res.json();
      toast.success(`Repository cloned and workspace "${data.workspaceName}" created`);
      setSelectedRepo("");
      setClonePath("");
      fetchIssues(); // Refresh to show new workspace
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Failed to clone repository");
    } finally {
      setCloning(false);
    }
  }

  const fetchIssues = useCallback(async () => {
    try {
      setLoadingIssues(true);
      const res = await fetch("/api/plugins/github/issues");
      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Failed to fetch issues");
      }
      const result = await res.json();
      setIssuesData(result);
    } catch (error) {
      console.error("Failed to fetch issues:", error);
    } finally {
      setLoadingIssues(false);
    }
  }, []);

  async function handleCreateTask(workspaceId: number, issueNumber: number) {
    const key = `${workspaceId}-${issueNumber}`;
    setCreatingTask(key);
    try {
      const res = await fetch("/api/plugins/github/issues/create-task", {
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

  return (
    <Tabs defaultValue="issues" className="space-y-6">
      <TabsList className="w-full sm:w-auto">
        <TabsTrigger value="issues" className="flex-1 sm:flex-initial">
          <CircleDot className="h-4 w-4 mr-2" />
          Issues
        </TabsTrigger>
        <TabsTrigger value="clone" className="flex-1 sm:flex-initial">
          <FolderGit2 className="h-4 w-4 mr-2" />
          Clone Repository
        </TabsTrigger>
        <TabsTrigger value="settings" className="flex-1 sm:flex-initial">
          Settings
        </TabsTrigger>
      </TabsList>

      {/* Issues Tab */}
      <TabsContent value="issues" className="space-y-6">
        {/* Connected Workspaces */}
        {issuesData && issuesData.workspaces.length > 0 && (
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <GitBranch className="h-5 w-5" />
                Connected Workspaces
              </CardTitle>
              <CardDescription>
                Workspaces with GitHub repositories configured
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-2">
                {issuesData.workspaces.map((workspace) => (
                  <div
                    key={workspace.workspaceId}
                    className="flex items-center justify-between p-3 rounded-lg bg-muted/30 border"
                  >
                    <div className="flex items-center gap-3">
                      <FolderGit2 className="h-5 w-5 text-muted-foreground" />
                      <div>
                        <p className="font-medium">{workspace.workspaceName}</p>
                        <p className="text-sm text-muted-foreground">
                          {workspace.owner}/{workspace.repo}
                        </p>
                      </div>
                    </div>
                    <a
                      href={`https://github.com/${workspace.owner}/${workspace.repo}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-muted-foreground hover:text-foreground"
                    >
                      <ExternalLink className="h-4 w-4" />
                    </a>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        )}

        {/* Issues List */}
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <CardTitle>Open Issues</CardTitle>
                <CardDescription>
                  {issuesData ? (
                    <>
                      {issuesData.totalIssues} open issues across{" "}
                      {issuesData.workspaces.length} workspaces
                      {issuesData.issuesWithTasks > 0 && (
                        <span className="ml-1">
                          ({issuesData.issuesWithTasks} already have tasks)
                        </span>
                      )}
                    </>
                  ) : (
                    "Loading..."
                  )}
                </CardDescription>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={fetchIssues}
                disabled={loadingIssues}
              >
                <RefreshCw className={`h-4 w-4 mr-1 ${loadingIssues ? "animate-spin" : ""}`} />
                Refresh
              </Button>
            </div>
          </CardHeader>
          <CardContent>
            {loadingIssues && !issuesData ? (
              <div className="flex items-center justify-center h-32 text-muted-foreground">
                <Loader2 className="h-6 w-6 animate-spin mr-2" />
                Loading issues...
              </div>
            ) : !issuesData || issuesData.workspaces.length === 0 ? (
              <div className="rounded-lg border border-dashed p-6 text-center text-muted-foreground">
                <p>No GitHub-connected workspaces found.</p>
                <p className="text-sm mt-2">
                  Clone a repository or configure GitHub for existing workspaces.
                </p>
              </div>
            ) : (
              <div className="space-y-6">
                {issuesData.workspaces.map((workspace) => (
                  <div key={workspace.workspaceId}>
                    <div className="flex items-center justify-between mb-3">
                      <h4 className="font-medium">{workspace.workspaceName}</h4>
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
            )}
          </CardContent>
        </Card>
      </TabsContent>

      {/* Clone Repository Tab */}
      <TabsContent value="clone" className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <FolderGit2 className="h-5 w-5" />
              Clone Repository
            </CardTitle>
            <CardDescription>
              Clone a GitHub repository and create a workspace
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            {connectionStatus !== "connected" ? (
              <div className="rounded-lg border border-dashed p-6 text-center text-muted-foreground">
                <AlertCircle className="h-8 w-8 mx-auto mb-2" />
                <p>GitHub is not connected.</p>
                <p className="text-sm mt-2">
                  Configure your GitHub token in the Settings tab first.
                </p>
              </div>
            ) : (
              <>
                <div className="space-y-2">
                  <Label>Repository</Label>
                  <div className="flex gap-2">
                    <Select value={selectedRepo} onValueChange={setSelectedRepo}>
                      <SelectTrigger className="flex-1">
                        <SelectValue placeholder="Select a repository" />
                      </SelectTrigger>
                      <SelectContent>
                        {repos.map((repo) => (
                          <SelectItem key={repo.fullName} value={repo.fullName}>
                            <div className="flex items-center gap-2">
                              <GitBranch className="h-4 w-4 text-muted-foreground" />
                              {repo.fullName}
                              {repo.private && (
                                <Badge variant="secondary" className="text-xs">
                                  Private
                                </Badge>
                              )}
                            </div>
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Button
                      variant="outline"
                      size="icon"
                      onClick={fetchRepos}
                      disabled={loadingRepos}
                    >
                      <RefreshCw className={`h-4 w-4 ${loadingRepos ? "animate-spin" : ""}`} />
                    </Button>
                  </div>
                </div>

                {selectedRepo && (
                  <>
                    <div className="space-y-2">
                      <Label htmlFor="clone-path">Local Path</Label>
                      <Input
                        id="clone-path"
                        value={clonePath}
                        onChange={(e) => setClonePath(e.target.value)}
                        placeholder={scopePath ? `${scopePath}/repo-name` : "/path/to/clone"}
                      />
                      <p className="text-xs text-muted-foreground">
                        Where to clone the repository
                      </p>
                    </div>

                    <Button onClick={handleClone} disabled={cloning} className="w-full">
                      {cloning ? (
                        <>
                          <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                          Cloning...
                        </>
                      ) : (
                        <>
                          <FolderGit2 className="h-4 w-4 mr-2" />
                          Clone & Create Workspace
                        </>
                      )}
                    </Button>
                  </>
                )}
              </>
            )}
          </CardContent>
        </Card>
      </TabsContent>

      {/* Settings Tab */}
      <TabsContent value="settings" className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle>GitHub Connection</CardTitle>
            <CardDescription>
              Configure your GitHub personal access token
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex items-center gap-2 mb-4">
              <span className="text-sm text-muted-foreground">Status:</span>
              {connectionStatus === "connected" && (
                <Badge variant="outline" className="text-green-600 border-green-600">
                  <CheckCircle2 className="h-3 w-3 mr-1" />
                  Connected
                </Badge>
              )}
              {connectionStatus === "error" && (
                <Badge variant="outline" className="text-red-600 border-red-600">
                  <AlertCircle className="h-3 w-3 mr-1" />
                  Error
                </Badge>
              )}
              {connectionStatus === "unknown" && !tokenSaved && (
                <Badge variant="outline" className="text-muted-foreground">
                  Not configured
                </Badge>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="github-token">GitHub Personal Access Token</Label>
              <div className="flex gap-2">
                <div className="relative flex-1">
                  <Input
                    id="github-token"
                    type={showToken ? "text" : "password"}
                    placeholder={tokenSaved ? "••••••••••••••••" : "ghp_xxxxxxxxxxxx"}
                    value={token}
                    onChange={(e) => setToken(e.target.value)}
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
                    onClick={() => setShowToken(!showToken)}
                  >
                    {showToken ? (
                      <EyeOff className="h-4 w-4 text-muted-foreground" />
                    ) : (
                      <Eye className="h-4 w-4 text-muted-foreground" />
                    )}
                  </Button>
                </div>
                <Button onClick={handleSaveToken} disabled={savingToken || !token.trim()}>
                  {savingToken ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : tokenSaved ? (
                    <>
                      <Check className="h-4 w-4 mr-1" />
                      Update
                    </>
                  ) : (
                    "Save"
                  )}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                Get a token at{" "}
                <a
                  href="https://github.com/settings/tokens"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-primary hover:underline"
                >
                  github.com/settings/tokens
                </a>
                {" "}with repo and issue permissions.
              </p>
            </div>
          </CardContent>
        </Card>
      </TabsContent>
    </Tabs>
  );
}
