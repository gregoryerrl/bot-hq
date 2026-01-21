"use client";

import { useState, useEffect } from "react";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2, Github, RefreshCw } from "lucide-react";
import { toast } from "sonner";

interface GitHubWorkspaceSettingsProps {
  workspaceId: number;
}

interface GitHubConfig {
  owner: string;
  repo: string;
}

export function GitHubWorkspaceSettings({ workspaceId }: GitHubWorkspaceSettingsProps) {
  const [config, setConfig] = useState<GitHubConfig>({ owner: "", repo: "" });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [syncing, setSyncing] = useState(false);

  useEffect(() => {
    async function fetchConfig() {
      try {
        const res = await fetch(`/api/plugins/github/workspace-data/${workspaceId}`);
        if (res.ok) {
          const data = await res.json();
          setConfig({
            owner: data.owner || "",
            repo: data.repo || "",
          });
        }
      } catch (error) {
        console.error("Failed to fetch GitHub config:", error);
      } finally {
        setLoading(false);
      }
    }

    fetchConfig();
  }, [workspaceId]);

  async function handleSave() {
    setSaving(true);
    try {
      const res = await fetch(`/api/plugins/github/workspace-data/${workspaceId}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(config),
      });

      if (!res.ok) throw new Error("Failed to save");
      toast.success("GitHub settings saved");
    } catch {
      toast.error("Failed to save GitHub settings");
    } finally {
      setSaving(false);
    }
  }

  async function handleSync() {
    if (!config.owner || !config.repo) {
      toast.error("Please configure owner and repo first");
      return;
    }

    setSyncing(true);
    try {
      const res = await fetch("/api/plugins/github/sync", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ workspaceId }),
      });

      if (!res.ok) throw new Error("Failed to sync");
      const data = await res.json();
      toast.success(`Synced ${data.issuesCount || 0} issues from GitHub`);
    } catch {
      toast.error("Failed to sync issues");
    } finally {
      setSyncing(false);
    }
  }

  if (loading) {
    return (
      <Card>
        <CardContent className="flex items-center justify-center p-8">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Github className="h-5 w-5" />
          GitHub Settings
        </CardTitle>
        <CardDescription>
          Connect this workspace to a GitHub repository
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="owner">Owner</Label>
            <Input
              id="owner"
              placeholder="e.g., octocat"
              value={config.owner}
              onChange={(e) => setConfig({ ...config, owner: e.target.value })}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="repo">Repository</Label>
            <Input
              id="repo"
              placeholder="e.g., hello-world"
              value={config.repo}
              onChange={(e) => setConfig({ ...config, repo: e.target.value })}
            />
          </div>
        </div>

        {config.owner && config.repo && (
          <p className="text-sm text-muted-foreground">
            Repository: <a
              href={`https://github.com/${config.owner}/${config.repo}`}
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary hover:underline"
            >
              github.com/{config.owner}/{config.repo}
            </a>
          </p>
        )}

        <div className="flex justify-between">
          <Button
            variant="outline"
            onClick={handleSync}
            disabled={syncing || !config.owner || !config.repo}
          >
            {syncing ? (
              <Loader2 className="h-4 w-4 mr-2 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4 mr-2" />
            )}
            Sync Issues
          </Button>
          <Button onClick={handleSave} disabled={saving}>
            {saving && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
            Save
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
