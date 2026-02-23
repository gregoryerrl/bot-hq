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
import { GitBranch, MoreVertical, Trash2, CheckCircle2, AlertCircle, Loader2, Scan, Plus } from "lucide-react";
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
  workspaceId?: string;
  onUpdate: () => void;
  onAddRemote?: () => void;
}

export function RemoteList({ workspaceId, onUpdate, onAddRemote }: RemoteListProps) {
  const [remotes, setRemotes] = useState<GitRemote[]>([]);
  const [loading, setLoading] = useState(true);
  const [detecting, setDetecting] = useState(false);

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
    } catch {
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
    } catch {
      toast.error("Failed to update remote");
    }
  }

  async function detectRemotes() {
    setDetecting(true);
    try {
      const body = wsId ? { workspaceId: wsId } : {};
      const res = await fetch("/api/git-remote/detect", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (res.ok) {
        const data = await res.json();
        if (data.totalDetected > 0) {
          toast.success(
            `Detected ${data.totalDetected} remote${data.totalDetected > 1 ? "s" : ""}${data.results.length > 1 ? ` across ${data.results.length} workspaces` : ""}`
          );
          fetchRemotes();
          onUpdate();
        } else {
          toast.info("No new remotes detected — remotes already configured");
        }
      } else {
        throw new Error("Detection failed");
      }
    } catch {
      toast.error("Failed to detect remotes");
    } finally {
      setDetecting(false);
    }
  }

  const wsId = workspaceId ? Number(workspaceId) : null;

  // Filter remotes based on selected workspace
  const filteredRemotes = wsId
    ? remotes.filter((r) => !r.workspaceId || r.workspaceId === wsId)
    : remotes;

  const globalRemotes = filteredRemotes.filter((r) => !r.workspaceId);
  const workspaceRemotes = filteredRemotes.filter((r) => r.workspaceId);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        <span className="ml-2 text-muted-foreground">Loading remotes...</span>
      </div>
    );
  }

  if (filteredRemotes.length === 0) {
    return (
      <Card>
        <CardContent className="py-12">
          <div className="text-center text-muted-foreground">
            <GitBranch className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p className="font-medium">
              {wsId ? "No Remotes for This Workspace" : "No Git Remotes Configured"}
            </p>
            <p className="text-sm mt-2 mb-4">
              {wsId
                ? "Detect remotes from the repository or add one manually."
                : "Add a remote to connect to GitHub, GitLab, Bitbucket, or other git providers."}
            </p>
            <div className="flex gap-2 justify-center">
              <Button
                variant="outline"
                size="sm"
                onClick={detectRemotes}
                disabled={detecting}
              >
                {detecting ? (
                  <Loader2 className="h-4 w-4 mr-1.5 animate-spin" />
                ) : (
                  <Scan className="h-4 w-4 mr-1.5" />
                )}
                {wsId ? "Detect Remotes" : "Detect from All Workspaces"}
              </Button>
              {onAddRemote && (
                <Button size="sm" onClick={onAddRemote}>
                  <Plus className="h-4 w-4 mr-1.5" />
                  Add Remote
                </Button>
              )}
            </div>
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={detectRemotes}
          disabled={detecting}
        >
          {detecting ? (
            <Loader2 className="h-4 w-4 mr-1.5 animate-spin" />
          ) : (
            <Scan className="h-4 w-4 mr-1.5" />
          )}
          {wsId ? "Detect Remotes" : "Detect from All Workspaces"}
        </Button>
        {onAddRemote && (
          <Button size="sm" onClick={onAddRemote}>
            <Plus className="h-4 w-4 mr-1.5" />
            Add Remote
          </Button>
        )}
      </div>

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
            <CardTitle>{wsId ? "Workspace Remote" : "Workspace Remotes"}</CardTitle>
            <CardDescription>
              {wsId
                ? "Remote linked to this workspace"
                : "Remotes linked to specific workspaces"}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {workspaceRemotes.map((remote) => (
              <RemoteItem
                key={remote.id}
                remote={remote}
                onDelete={() => deleteRemote(remote.id)}
                onSetDefault={() => setDefaultRemote(remote.id)}
                showWorkspace={!wsId}
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
