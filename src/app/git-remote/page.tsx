"use client";

import { useState, useEffect, useCallback } from "react";
import { Header } from "@/components/layout/header";
import { RemoteList } from "@/components/git-remote/remote-list";
import { RemoteForm } from "@/components/git-remote/remote-form";
import { IssuesList } from "@/components/git-remote/issues-list";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Button } from "@/components/ui/button";
import { Plus, GitBranch, CircleDot, FolderGit2 } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export default function GitRemotePage() {
  const [showAddRemote, setShowAddRemote] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  const handleRemoteAdded = () => {
    setShowAddRemote(false);
    setRefreshKey(k => k + 1);
  };

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Git Remotes"
        description="Manage git provider connections and sync issues"
      />
      <div className="flex-1 p-4 md:p-6">
        <Tabs defaultValue="remotes" className="space-y-6">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <TabsList className="w-full sm:w-auto">
              <TabsTrigger value="remotes" className="flex-1 sm:flex-initial">
                <GitBranch className="h-4 w-4 mr-2" />
                Remotes
              </TabsTrigger>
              <TabsTrigger value="issues" className="flex-1 sm:flex-initial">
                <CircleDot className="h-4 w-4 mr-2" />
                Issues
              </TabsTrigger>
              <TabsTrigger value="clone" className="flex-1 sm:flex-initial">
                <FolderGit2 className="h-4 w-4 mr-2" />
                Clone
              </TabsTrigger>
            </TabsList>
            <Button onClick={() => setShowAddRemote(true)} size="sm">
              <Plus className="h-4 w-4 mr-1" />
              Add Remote
            </Button>
          </div>

          <TabsContent value="remotes">
            <RemoteList key={refreshKey} onUpdate={() => setRefreshKey(k => k + 1)} />
          </TabsContent>

          <TabsContent value="issues">
            <IssuesList key={refreshKey} />
          </TabsContent>

          <TabsContent value="clone">
            <CloneRepository onCloned={() => setRefreshKey(k => k + 1)} />
          </TabsContent>
        </Tabs>
      </div>

      <Dialog open={showAddRemote} onOpenChange={setShowAddRemote}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>Add Git Remote</DialogTitle>
          </DialogHeader>
          <RemoteForm onSuccess={handleRemoteAdded} onCancel={() => setShowAddRemote(false)} />
        </DialogContent>
      </Dialog>
    </div>
  );
}

function CloneRepository({ onCloned }: { onCloned: () => void }) {
  const [repos, setRepos] = useState<Array<{ owner: string; name: string; fullName: string; description: string | null; private: boolean }>>([]);
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
        // Check if there's a GitHub remote with credentials
        const githubRemote = remotes.find((r: any) => r.provider === "github" && r.hasCredentials);
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

  useEffect(() => {
    if (selectedRepo && scopePath) {
      const repo = repos.find(r => r.fullName === selectedRepo);
      if (repo) {
        setClonePath(`${scopePath}/${repo.name}`);
      }
    }
  }, [selectedRepo, scopePath, repos]);

  async function handleClone() {
    if (!selectedRepo) return;

    const repo = repos.find(r => r.fullName === selectedRepo);
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

      setSelectedRepo("");
      setClonePath("");
      onCloned();
    } catch (error) {
      console.error("Clone failed:", error);
    } finally {
      setCloning(false);
    }
  }

  if (!hasToken) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
        <FolderGit2 className="h-12 w-12 mx-auto mb-4 opacity-50" />
        <p className="font-medium mb-2">No Git Remote Configured</p>
        <p className="text-sm mb-4">
          Add a GitHub remote with credentials to clone repositories.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="rounded-lg border p-4">
        <h3 className="font-medium mb-4">Clone Repository</h3>
        <div className="space-y-4">
          <div>
            <label className="text-sm font-medium mb-2 block">Repository</label>
            <select
              className="w-full p-2 border rounded-md bg-background"
              value={selectedRepo}
              onChange={(e) => setSelectedRepo(e.target.value)}
              disabled={loading}
            >
              <option value="">Select a repository...</option>
              {repos.map((repo) => (
                <option key={repo.fullName} value={repo.fullName}>
                  {repo.fullName} {repo.private ? "(Private)" : ""}
                </option>
              ))}
            </select>
            {loading && <p className="text-sm text-muted-foreground mt-1">Loading repositories...</p>}
          </div>

          {selectedRepo && (
            <>
              <div>
                <label className="text-sm font-medium mb-2 block">Local Path</label>
                <input
                  type="text"
                  className="w-full p-2 border rounded-md bg-background"
                  value={clonePath}
                  onChange={(e) => setClonePath(e.target.value)}
                  placeholder={scopePath ? `${scopePath}/repo-name` : "/path/to/clone"}
                />
              </div>

              <Button onClick={handleClone} disabled={cloning}>
                {cloning ? "Cloning..." : "Clone & Create Workspace"}
              </Button>
            </>
          )}
        </div>
      </div>

      <Button variant="outline" onClick={fetchRepos} disabled={loading}>
        Refresh Repositories
      </Button>
    </div>
  );
}
