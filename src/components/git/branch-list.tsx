"use client";

import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  GitBranch,
  Plus,
  Trash2,
  ArrowRight,
  RefreshCw,
} from "lucide-react";

interface Branch {
  name: string;
  hash: string;
  date: string;
  message: string;
  current: boolean;
  isRemote: boolean;
}

interface BranchData {
  current: string;
  local: Branch[];
  remote: Branch[];
}

interface BranchListProps {
  workspaceId: string;
}

export function BranchList({ workspaceId }: BranchListProps) {
  const [data, setData] = useState<BranchData | null>(null);
  const [loading, setLoading] = useState(true);
  const [showCreate, setShowCreate] = useState(false);
  const [newBranchName, setNewBranchName] = useState("");
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  const fetchBranches = useCallback(async () => {
    if (!workspaceId) return;
    setLoading(true);
    try {
      const res = await fetch(`/api/git/branches?workspaceId=${workspaceId}`);
      if (res.ok) setData(await res.json());
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    fetchBranches();
  }, [fetchBranches]);

  async function handleSwitch(branch: string) {
    setActionLoading(`switch-${branch}`);
    await fetch("/api/git/branches/switch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId), branch }),
    });
    setActionLoading(null);
    fetchBranches();
  }

  async function handleCreate() {
    if (!newBranchName.trim()) return;
    setActionLoading("create");
    await fetch("/api/git/branches", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId), name: newBranchName }),
    });
    setActionLoading(null);
    setNewBranchName("");
    setShowCreate(false);
    fetchBranches();
  }

  async function handleDelete(branch: string) {
    setActionLoading(`delete-${branch}`);
    await fetch("/api/git/branches", {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ workspaceId: Number(workspaceId), name: branch }),
    });
    setActionLoading(null);
    fetchBranches();
  }

  if (loading && !data) {
    return <div className="text-sm text-muted-foreground p-4">Loading branches...</div>;
  }

  if (!data) {
    return <div className="text-sm text-muted-foreground p-4">Failed to load branches</div>;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium">Local Branches</h4>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" onClick={fetchBranches}>
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
          <Button size="sm" variant="outline" onClick={() => setShowCreate(!showCreate)}>
            <Plus className="h-3.5 w-3.5 mr-1" />
            New Branch
          </Button>
        </div>
      </div>

      {showCreate && (
        <div className="flex gap-2">
          <Input
            placeholder="Branch name..."
            value={newBranchName}
            onChange={(e) => setNewBranchName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCreate()}
            className="flex-1"
          />
          <Button
            size="sm"
            onClick={handleCreate}
            disabled={!newBranchName.trim() || actionLoading === "create"}
          >
            Create
          </Button>
        </div>
      )}

      <div className="rounded-lg border divide-y">
        {data.local.map((branch) => (
          <div
            key={branch.name}
            className="flex items-center justify-between px-3 py-2 text-sm"
          >
            <div className="flex items-center gap-2 min-w-0 flex-1">
              <GitBranch className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
              <span className="font-mono text-xs truncate">{branch.name}</span>
              {branch.current && (
                <Badge variant="default" className="text-[10px] px-1.5 py-0">
                  current
                </Badge>
              )}
            </div>
            <div className="flex items-center gap-2 shrink-0 ml-2">
              <span className="text-xs text-muted-foreground hidden sm:block">
                {branch.date}
              </span>
              {!branch.current && (
                <>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2"
                    onClick={() => handleSwitch(branch.name)}
                    disabled={actionLoading === `switch-${branch.name}`}
                  >
                    <ArrowRight className="h-3 w-3" />
                  </Button>
                  {branch.name !== "main" && branch.name !== "master" && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-destructive hover:text-destructive"
                      onClick={() => handleDelete(branch.name)}
                      disabled={actionLoading === `delete-${branch.name}`}
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  )}
                </>
              )}
            </div>
          </div>
        ))}
        {data.local.length === 0 && (
          <div className="px-3 py-4 text-sm text-muted-foreground text-center">
            No local branches
          </div>
        )}
      </div>

      {data.remote.length > 0 && (
        <>
          <h4 className="text-sm font-medium">Remote Branches</h4>
          <div className="rounded-lg border divide-y">
            {data.remote.map((branch) => (
              <div
                key={branch.name}
                className="flex items-center justify-between px-3 py-2 text-sm"
              >
                <div className="flex items-center gap-2 min-w-0">
                  <GitBranch className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                  <span className="font-mono text-xs truncate text-muted-foreground">
                    {branch.name}
                  </span>
                </div>
                <span className="text-xs text-muted-foreground shrink-0">
                  {branch.date}
                </span>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
