"use client";

import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { FolderGit2, X } from "lucide-react";
import { toast } from "sonner";

interface RemoteInfo {
  gitName: string;
  provider: string;
  owner: string | null;
  repo: string | null;
}

interface WorkspaceSuggestionCardProps {
  name: string;
  repoPath: string;
  remotes?: RemoteInfo[];
  onAccepted: () => void;
  onDismissed: () => void;
}

const PROVIDER_LABELS: Record<string, string> = {
  github: "GitHub",
  gitlab: "GitLab",
  bitbucket: "Bitbucket",
  custom: "Git",
};

export function WorkspaceSuggestionCard({
  name,
  repoPath,
  remotes,
  onAccepted,
  onDismissed,
}: WorkspaceSuggestionCardProps) {
  async function handleAdd() {
    try {
      const res = await fetch("/api/workspaces/discover", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action: "add_workspace", name, repoPath }),
      });

      if (!res.ok) {
        const data = await res.json();
        throw new Error(data.error || "Failed to add workspace");
      }

      const data = await res.json();
      const remoteCount = data.autoDetectedRemotes || 0;
      const remoteMsg = remoteCount > 0
        ? ` with ${remoteCount} remote${remoteCount > 1 ? "s" : ""} detected`
        : "";
      toast.success(`Workspace "${name}" added${remoteMsg}`);
      onAccepted();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Failed to add workspace");
    }
  }

  const validRemotes = remotes?.filter((r) => r.owner && r.repo) || [];

  return (
    <Card>
      <CardContent className="flex items-center justify-between py-3 px-4">
        <div className="flex items-center gap-3 min-w-0">
          <FolderGit2 className="h-5 w-5 text-green-600 flex-shrink-0" />
          <div className="min-w-0">
            <p className="font-medium truncate">{name}</p>
            <p className="text-xs text-muted-foreground truncate">{repoPath}</p>
            {validRemotes.length > 0 && (
              <div className="flex flex-wrap gap-1 mt-1">
                {validRemotes.map((r, i) => (
                  <Badge key={i} variant="outline" className="text-xs">
                    {PROVIDER_LABELS[r.provider] || r.provider}: {r.owner}/{r.repo}
                  </Badge>
                ))}
              </div>
            )}
          </div>
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          <Button size="sm" onClick={handleAdd}>
            Add Workspace
          </Button>
          <Button size="sm" variant="ghost" onClick={onDismissed}>
            <X className="h-4 w-4" />
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
