"use client";

import { useState, useEffect } from "react";
import { Header } from "@/components/layout/header";
import { WorkspaceGitSelector } from "@/components/git/workspace-git-selector";
import { StatusView } from "@/components/git/status-view";
import { BranchList } from "@/components/git/branch-list";
import { CommitHistory } from "@/components/git/commit-history";
import { StashList } from "@/components/git/stash-list";
import { RemoteList } from "@/components/git-remote/remote-list";
import { RemoteForm } from "@/components/git-remote/remote-form";
import { IssuesList } from "@/components/git-remote/issues-list";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Plus,
  FileText,
  GitBranch,
  History,
  Package,
  Globe,
  CircleDot,
  FolderGit2,
  Loader2,
  Download,
} from "lucide-react";
import { toast } from "sonner";

export default function GitPage() {
  const [workspaceId, setWorkspaceId] = useState("");
  const [showAddRemote, setShowAddRemote] = useState(false);
  const [showClone, setShowClone] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  const handleRemoteAdded = () => {
    setShowAddRemote(false);
    setRefreshKey((k) => k + 1);
  };

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Git"
        description="Full git management — status, branches, commits, stash, and remotes"
      />
      <div className="flex-1 p-4 md:p-6">
        <Tabs defaultValue="status" className="space-y-6">
          <div className="flex flex-col gap-4">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
              <WorkspaceGitSelector
                value={workspaceId}
                onChange={setWorkspaceId}
              />
              <Button
                onClick={() => setShowClone(true)}
                size="sm"
                variant="outline"
              >
                <Download className="h-4 w-4 mr-1" />
                Clone Repo
              </Button>
            </div>
            <TabsList className="w-full sm:w-auto flex-wrap">
              <TabsTrigger value="status" className="flex-1 sm:flex-initial">
                <FileText className="h-4 w-4 mr-1.5" />
                Status
              </TabsTrigger>
              <TabsTrigger value="branches" className="flex-1 sm:flex-initial">
                <GitBranch className="h-4 w-4 mr-1.5" />
                Branches
              </TabsTrigger>
              <TabsTrigger value="history" className="flex-1 sm:flex-initial">
                <History className="h-4 w-4 mr-1.5" />
                History
              </TabsTrigger>
              <TabsTrigger value="stash" className="flex-1 sm:flex-initial">
                <Package className="h-4 w-4 mr-1.5" />
                Stash
              </TabsTrigger>
              <TabsTrigger value="remotes" className="flex-1 sm:flex-initial">
                <Globe className="h-4 w-4 mr-1.5" />
                Remotes
              </TabsTrigger>
              <TabsTrigger value="issues" className="flex-1 sm:flex-initial">
                <CircleDot className="h-4 w-4 mr-1.5" />
                Issues
              </TabsTrigger>
            </TabsList>
          </div>

          <TabsContent value="status">
            {workspaceId ? (
              <StatusView key={`status-${workspaceId}-${refreshKey}`} workspaceId={workspaceId} />
            ) : (
              <EmptyWorkspace />
            )}
          </TabsContent>

          <TabsContent value="branches">
            {workspaceId ? (
              <BranchList key={`branches-${workspaceId}-${refreshKey}`} workspaceId={workspaceId} />
            ) : (
              <EmptyWorkspace />
            )}
          </TabsContent>

          <TabsContent value="history">
            {workspaceId ? (
              <CommitHistory key={`history-${workspaceId}-${refreshKey}`} workspaceId={workspaceId} />
            ) : (
              <EmptyWorkspace />
            )}
          </TabsContent>

          <TabsContent value="stash">
            {workspaceId ? (
              <StashList key={`stash-${workspaceId}-${refreshKey}`} workspaceId={workspaceId} />
            ) : (
              <EmptyWorkspace />
            )}
          </TabsContent>

          <TabsContent value="remotes">
            <RemoteList
              key={`remotes-${workspaceId}-${refreshKey}`}
              workspaceId={workspaceId}
              onUpdate={() => setRefreshKey((k) => k + 1)}
              onAddRemote={() => setShowAddRemote(true)}
            />
          </TabsContent>

          <TabsContent value="issues">
            <IssuesList
              key={`issues-${workspaceId}-${refreshKey}`}
              workspaceId={workspaceId}
            />
          </TabsContent>
        </Tabs>
      </div>

      <Dialog open={showAddRemote} onOpenChange={setShowAddRemote}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Add Git Remote</DialogTitle>
          </DialogHeader>
          <RemoteForm
            onSuccess={handleRemoteAdded}
            onCancel={() => setShowAddRemote(false)}
          />
        </DialogContent>
      </Dialog>

      <Dialog open={showClone} onOpenChange={setShowClone}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Clone Repository</DialogTitle>
          </DialogHeader>
          <CloneRepository
            onCloned={() => {
              setShowClone(false);
              setRefreshKey((k) => k + 1);
            }}
          />
        </DialogContent>
      </Dialog>
    </div>
  );
}

function EmptyWorkspace() {
  return (
    <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
      <FolderGit2 className="h-12 w-12 mx-auto mb-4 opacity-50" />
      <p className="font-medium mb-1">No Workspace Selected</p>
      <p className="text-sm">Select a workspace above to view git status.</p>
    </div>
  );
}

function CloneRepository({ onCloned }: { onCloned: () => void }) {
  const [repos, setRepos] = useState<
    Array<{
      owner: string;
      name: string;
      fullName: string;
      description: string | null;
      private: boolean;
    }>
  >([]);
  const [loading, setLoading] = useState(false);
  const [selectedRepo, setSelectedRepo] = useState("");
  const [clonePath, setClonePath] = useState("");
  const [cloning, setCloning] = useState(false);
  const [scopePath, setScopePath] = useState("");
  const [hasToken, setHasToken] = useState(false);

  useEffect(() => {
    loadSettings();
  }, []);

  async function loadSettings() {
    try {
      const [settingsRes, remotesRes] = await Promise.all([
        fetch("/api/settings"),
        fetch("/api/git-remote"),
      ]);

      if (settingsRes.ok) {
        const data = await settingsRes.json();
        setScopePath(data.scopePath || "");
      }

      if (remotesRes.ok) {
        const remotes = await remotesRes.json();
        const githubRemote = remotes.find(
          (r: { provider: string; hasCredentials: boolean }) =>
            r.provider === "github" && r.hasCredentials
        );
        setHasToken(!!githubRemote);
        if (githubRemote) {
          fetchRepos();
        }
      }
    } catch (error) {
      console.error("Failed to load settings:", error);
    }
  }

  async function fetchRepos() {
    setLoading(true);
    try {
      const res = await fetch("/api/git-remote/repos");
      if (res.ok) {
        const data = await res.json();
        setRepos(data.repos || []);
      }
    } catch (error) {
      console.error("Failed to fetch repos:", error);
    } finally {
      setLoading(false);
    }
  }

  async function handleClone() {
    if (!selectedRepo) return;
    const repo = repos.find((r) => r.fullName === selectedRepo);
    if (!repo) return;

    setCloning(true);
    try {
      const res = await fetch("/api/git-remote/clone", {
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
        throw new Error(error.error || "Failed to clone");
      }

      toast.success(`Cloned ${repo.fullName} successfully`);
      setSelectedRepo("");
      setClonePath("");
      onCloned();
    } catch (error) {
      console.error("Clone failed:", error);
      toast.error(error instanceof Error ? error.message : "Clone failed");
    } finally {
      setCloning(false);
    }
  }

  if (!hasToken) {
    return (
      <div className="text-center text-muted-foreground py-4">
        <FolderGit2 className="h-10 w-10 mx-auto mb-3 opacity-50" />
        <p className="font-medium mb-1">No GitHub Token</p>
        <p className="text-sm">
          Add a GitHub remote with credentials to clone repositories.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div>
        <label className="text-sm font-medium mb-2 block">Repository</label>
        <select
          className="w-full p-2 border rounded-md bg-background"
          value={selectedRepo}
          onChange={(e) => {
            setSelectedRepo(e.target.value);
            const repo = repos.find((r) => r.fullName === e.target.value);
            if (repo && scopePath) {
              setClonePath(`${scopePath}/${repo.name}`);
            }
          }}
          disabled={loading}
        >
          <option value="">Select a repository...</option>
          {repos.map((repo) => (
            <option key={repo.fullName} value={repo.fullName}>
              {repo.fullName} {repo.private ? "(Private)" : ""}
            </option>
          ))}
        </select>
        {loading && (
          <p className="text-sm text-muted-foreground mt-1">
            Loading repositories...
          </p>
        )}
      </div>

      {selectedRepo && (
        <div>
          <label className="text-sm font-medium mb-2 block">
            Local Path
          </label>
          <input
            type="text"
            className="w-full p-2 border rounded-md bg-background"
            value={clonePath}
            onChange={(e) => setClonePath(e.target.value)}
            placeholder={
              scopePath ? `${scopePath}/repo-name` : "/path/to/clone"
            }
          />
        </div>
      )}

      <div className="flex gap-3 pt-2">
        <Button onClick={handleClone} disabled={cloning || !selectedRepo}>
          {cloning && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
          {cloning ? "Cloning..." : "Clone & Create Workspace"}
        </Button>
        <Button variant="outline" onClick={fetchRepos} disabled={loading}>
          {loading && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
          Refresh
        </Button>
      </div>
    </div>
  );
}
