"use client";

import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
  ArrowUp,
  ArrowDown,
  RefreshCw,
  GitCommit,
  Package,
} from "lucide-react";

interface StatusData {
  branch: string;
  upstream: string;
  ahead: number;
  behind: number;
  staged: string[];
  modified: string[];
  untracked: string[];
  clean: boolean;
  stashCount: number;
}

interface StatusViewProps {
  workspaceId: string;
}

export function StatusView({ workspaceId }: StatusViewProps) {
  const [status, setStatus] = useState<StatusData | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectedStage, setSelectedStage] = useState<Set<string>>(new Set());
  const [selectedUnstage, setSelectedUnstage] = useState<Set<string>>(new Set());
  const [commitMsg, setCommitMsg] = useState("");
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    if (!workspaceId) return;
    setLoading(true);
    try {
      const res = await fetch(`/api/git/status?workspaceId=${workspaceId}`);
      if (res.ok) {
        setStatus(await res.json());
        setSelectedStage(new Set());
        setSelectedUnstage(new Set());
      }
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    fetchStatus();
  }, [fetchStatus]);

  async function handleStage(files: string[]) {
    setActionLoading("stage");
    await fetch("/api/git/stage", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId), files, action: "stage" }),
    });
    setActionLoading(null);
    fetchStatus();
  }

  async function handleUnstage(files: string[]) {
    setActionLoading("unstage");
    await fetch("/api/git/stage", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId), files, action: "unstage" }),
    });
    setActionLoading(null);
    fetchStatus();
  }

  async function handleCommit() {
    if (!commitMsg.trim()) return;
    setActionLoading("commit");
    const res = await fetch("/api/git/commit", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId), message: commitMsg }),
    });
    setActionLoading(null);
    if (res.ok) {
      setCommitMsg("");
      fetchStatus();
    }
  }

  async function handlePush() {
    setActionLoading("push");
    await fetch("/api/git/push", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId), setUpstream: true }),
    });
    setActionLoading(null);
    fetchStatus();
  }

  async function handlePull() {
    setActionLoading("pull");
    await fetch("/api/git/pull", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId) }),
    });
    setActionLoading(null);
    fetchStatus();
  }

  if (loading && !status) {
    return <div className="text-sm text-muted-foreground p-4">Loading status...</div>;
  }

  if (!status) {
    return <div className="text-sm text-muted-foreground p-4">Failed to load status</div>;
  }

  const unstaged = [...status.modified, ...status.untracked];

  return (
    <div className="space-y-4">
      {/* Branch info bar */}
      <div className="flex items-center gap-3 flex-wrap">
        <Badge variant="outline" className="text-sm">
          {status.branch}
        </Badge>
        {status.upstream && (
          <span className="text-xs text-muted-foreground">{status.upstream}</span>
        )}
        {status.ahead > 0 && (
          <Badge variant="secondary" className="text-xs">
            <ArrowUp className="h-3 w-3 mr-1" />
            {status.ahead}
          </Badge>
        )}
        {status.behind > 0 && (
          <Badge variant="secondary" className="text-xs">
            <ArrowDown className="h-3 w-3 mr-1" />
            {status.behind}
          </Badge>
        )}
        {status.stashCount > 0 && (
          <Badge variant="secondary" className="text-xs">
            <Package className="h-3 w-3 mr-1" />
            {status.stashCount} stash{status.stashCount > 1 ? "es" : ""}
          </Badge>
        )}
        <Button variant="ghost" size="sm" onClick={fetchStatus} className="ml-auto">
          <RefreshCw className="h-3.5 w-3.5" />
        </Button>
      </div>

      {status.clean ? (
        <div className="rounded-lg border border-dashed p-6 text-center text-muted-foreground">
          Working tree clean
        </div>
      ) : (
        <>
          {/* Staged files */}
          {status.staged.length > 0 && (
            <div className="rounded-lg border p-3 space-y-2">
              <div className="flex items-center justify-between">
                <h4 className="text-sm font-medium text-green-600 dark:text-green-400">
                  Staged ({status.staged.length})
                </h4>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleUnstage(
                    selectedUnstage.size > 0
                      ? Array.from(selectedUnstage)
                      : status.staged
                  )}
                  disabled={actionLoading === "unstage"}
                >
                  {selectedUnstage.size > 0 ? "Unstage Selected" : "Unstage All"}
                </Button>
              </div>
              <div className="space-y-1">
                {status.staged.map((file) => (
                  <label key={file} className="flex items-center gap-2 text-sm hover:bg-muted/50 rounded px-2 py-0.5 cursor-pointer">
                    <Checkbox
                      checked={selectedUnstage.has(file)}
                      onCheckedChange={(checked) => {
                        const next = new Set(selectedUnstage);
                        if (checked) next.add(file);
                        else next.delete(file);
                        setSelectedUnstage(next);
                      }}
                    />
                    <span className="text-green-600 dark:text-green-400 font-mono text-xs truncate">
                      {file}
                    </span>
                  </label>
                ))}
              </div>
            </div>
          )}

          {/* Modified + Untracked files */}
          {unstaged.length > 0 && (
            <div className="rounded-lg border p-3 space-y-2">
              <div className="flex items-center justify-between">
                <h4 className="text-sm font-medium">
                  Changes ({status.modified.length} modified, {status.untracked.length} untracked)
                </h4>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleStage(
                    selectedStage.size > 0
                      ? Array.from(selectedStage)
                      : unstaged
                  )}
                  disabled={actionLoading === "stage"}
                >
                  {selectedStage.size > 0 ? "Stage Selected" : "Stage All"}
                </Button>
              </div>
              <div className="space-y-1">
                {status.modified.map((file) => (
                  <label key={file} className="flex items-center gap-2 text-sm hover:bg-muted/50 rounded px-2 py-0.5 cursor-pointer">
                    <Checkbox
                      checked={selectedStage.has(file)}
                      onCheckedChange={(checked) => {
                        const next = new Set(selectedStage);
                        if (checked) next.add(file);
                        else next.delete(file);
                        setSelectedStage(next);
                      }}
                    />
                    <span className="text-yellow-600 dark:text-yellow-400 font-mono text-xs truncate">
                      {file}
                    </span>
                  </label>
                ))}
                {status.untracked.map((file) => (
                  <label key={file} className="flex items-center gap-2 text-sm hover:bg-muted/50 rounded px-2 py-0.5 cursor-pointer">
                    <Checkbox
                      checked={selectedStage.has(file)}
                      onCheckedChange={(checked) => {
                        const next = new Set(selectedStage);
                        if (checked) next.add(file);
                        else next.delete(file);
                        setSelectedStage(next);
                      }}
                    />
                    <span className="text-muted-foreground font-mono text-xs truncate">
                      {file}
                    </span>
                  </label>
                ))}
              </div>
            </div>
          )}
        </>
      )}

      {/* Commit form */}
      {status.staged.length > 0 && (
        <div className="flex gap-2">
          <input
            type="text"
            placeholder="Commit message..."
            value={commitMsg}
            onChange={(e) => setCommitMsg(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCommit()}
            className="flex-1 h-9 rounded-md border border-input bg-transparent px-3 text-sm"
          />
          <Button
            size="sm"
            onClick={handleCommit}
            disabled={!commitMsg.trim() || actionLoading === "commit"}
          >
            <GitCommit className="h-3.5 w-3.5 mr-1" />
            Commit
          </Button>
        </div>
      )}

      {/* Push/Pull buttons */}
      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={handlePush}
          disabled={actionLoading === "push"}
        >
          <ArrowUp className="h-3.5 w-3.5 mr-1" />
          Push
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={handlePull}
          disabled={actionLoading === "pull"}
        >
          <ArrowDown className="h-3.5 w-3.5 mr-1" />
          Pull
        </Button>
      </div>
    </div>
  );
}
