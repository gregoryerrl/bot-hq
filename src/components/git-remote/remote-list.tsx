"use client";

import { useState, useEffect } from "react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { GitBranch, MoreVertical, Trash2, Settings, CheckCircle2, AlertCircle, Loader2 } from "lucide-react";
import { toast } from "sonner";

interface GitRemote {
  id: number;
  workspaceId: number | null;
  provider: string;
  name: string;
  url: string;
  owner: string | null;
  repo: string | null;
  isDefault: boolean;
  hasCredentials: boolean;
  workspaceName?: string;
  createdAt: string;
}

interface RemoteListProps {
  onUpdate: () => void;
}

export function RemoteList({ onUpdate }: RemoteListProps) {
  const [remotes, setRemotes] = useState<GitRemote[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchRemotes();
  }, []);

  async function fetchRemotes() {
    try {
      const res = await fetch("/api/git-remote");
      if (res.ok) {
        const data = await res.json();
        setRemotes(data);
      }
    } catch (error) {
      console.error("Failed to fetch remotes:", error);
      toast.error("Failed to load remotes");
    } finally {
      setLoading(false);
    }
  }

  async function deleteRemote(id: number) {
    if (!confirm("Are you sure you want to delete this remote?")) return;

    try {
      const res = await fetch(`/api/git-remote/${id}`, { method: "DELETE" });
      if (res.ok) {
        toast.success("Remote deleted");
        fetchRemotes();
        onUpdate();
      } else {
        throw new Error("Failed to delete");
      }
    } catch (error) {
      toast.error("Failed to delete remote");
    }
  }

  async function setDefaultRemote(id: number) {
    try {
      const res = await fetch(`/api/git-remote/${id}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ isDefault: true }),
      });
      if (res.ok) {
        toast.success("Default remote updated");
        fetchRemotes();
        onUpdate();
      }
    } catch (error) {
      toast.error("Failed to update remote");
    }
  }

  const providerIcons: Record<string, string> = {
    github: "GitHub",
    gitlab: "GitLab",
    bitbucket: "Bitbucket",
    gitea: "Gitea",
    custom: "Custom",
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        <span className="ml-2 text-muted-foreground">Loading remotes...</span>
      </div>
    );
  }

  if (remotes.length === 0) {
    return (
      <Card>
        <CardContent className="py-12">
          <div className="text-center text-muted-foreground">
            <GitBranch className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p className="font-medium">No Git Remotes Configured</p>
            <p className="text-sm mt-2">
              Add a remote to connect to GitHub, GitLab, Bitbucket, or other git providers.
            </p>
          </div>
        </CardContent>
      </Card>
    );
  }

  // Group by workspace
  const globalRemotes = remotes.filter(r => !r.workspaceId);
  const workspaceRemotes = remotes.filter(r => r.workspaceId);

  return (
    <div className="space-y-6">
      {globalRemotes.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Global Remotes</CardTitle>
            <CardDescription>
              Available to all workspaces for authentication
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {globalRemotes.map((remote) => (
              <RemoteItem
                key={remote.id}
                remote={remote}
                onDelete={() => deleteRemote(remote.id)}
                onSetDefault={() => setDefaultRemote(remote.id)}
              />
            ))}
          </CardContent>
        </Card>
      )}

      {workspaceRemotes.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Workspace Remotes</CardTitle>
            <CardDescription>
              Remotes linked to specific workspaces
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {workspaceRemotes.map((remote) => (
              <RemoteItem
                key={remote.id}
                remote={remote}
                onDelete={() => deleteRemote(remote.id)}
                onSetDefault={() => setDefaultRemote(remote.id)}
                showWorkspace
              />
            ))}
          </CardContent>
        </Card>
      )}
    </div>
  );
}

function RemoteItem({
  remote,
  onDelete,
  onSetDefault,
  showWorkspace = false,
}: {
  remote: GitRemote;
  onDelete: () => void;
  onSetDefault: () => void;
  showWorkspace?: boolean;
}) {
  return (
    <div className="flex items-center justify-between p-3 rounded-lg border bg-muted/30">
      <div className="flex items-center gap-3 min-w-0">
        <GitBranch className="h-5 w-5 text-muted-foreground shrink-0" />
        <div className="min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-medium truncate">{remote.name}</span>
            <Badge variant="outline" className="text-xs">
              {remote.provider}
            </Badge>
            {remote.isDefault && (
              <Badge variant="secondary" className="text-xs">
                Default
              </Badge>
            )}
            {remote.hasCredentials ? (
              <Badge variant="outline" className="text-xs text-green-600 border-green-600">
                <CheckCircle2 className="h-3 w-3 mr-1" />
                Connected
              </Badge>
            ) : (
              <Badge variant="outline" className="text-xs text-yellow-600 border-yellow-600">
                <AlertCircle className="h-3 w-3 mr-1" />
                No Token
              </Badge>
            )}
          </div>
          <div className="text-sm text-muted-foreground truncate">
            {remote.owner && remote.repo
              ? `${remote.owner}/${remote.repo}`
              : remote.url}
          </div>
          {showWorkspace && remote.workspaceName && (
            <div className="text-xs text-muted-foreground">
              Workspace: {remote.workspaceName}
            </div>
          )}
        </div>
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" className="shrink-0">
            <MoreVertical className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {!remote.isDefault && (
            <DropdownMenuItem onClick={onSetDefault}>
              <CheckCircle2 className="h-4 w-4 mr-2" />
              Set as Default
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onClick={onDelete} className="text-destructive">
            <Trash2 className="h-4 w-4 mr-2" />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
