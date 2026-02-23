"use client";

import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Package,
  Plus,
  Play,
  Copy,
  Trash2,
  RefreshCw,
} from "lucide-react";

interface Stash {
  index: string;
  message: string;
  date: string;
}

interface StashListProps {
  workspaceId: string;
}

export function StashList({ workspaceId }: StashListProps) {
  const [stashes, setStashes] = useState<Stash[]>([]);
  const [loading, setLoading] = useState(true);
  const [showSave, setShowSave] = useState(false);
  const [stashMsg, setStashMsg] = useState("");
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  const fetchStashes = useCallback(async () => {
    if (!workspaceId) return;
    setLoading(true);
    try {
      const res = await fetch(`/api/git/stash?workspaceId=${workspaceId}`);
      if (res.ok) {
        const data = await res.json();
        setStashes(data.stashes || []);
      }
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    fetchStashes();
  }, [fetchStashes]);

  async function stashAction(action: string, index?: number) {
    setActionLoading(`${action}-${index ?? "new"}`);
    await fetch("/api/git/stash", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        workspaceId: Number(workspaceId),
        action,
        index,
        message: action === "save" ? stashMsg || undefined : undefined,
      }),
    });
    setActionLoading(null);
    setStashMsg("");
    setShowSave(false);
    fetchStashes();
  }

  if (loading && stashes.length === 0 && !showSave) {
    return <div className="text-sm text-muted-foreground p-4">Loading stashes...</div>;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium">Stashes</h4>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" onClick={fetchStashes}>
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => setShowSave(!showSave)}
          >
            <Plus className="h-3.5 w-3.5 mr-1" />
            Stash Changes
          </Button>
        </div>
      </div>

      {showSave && (
        <div className="flex gap-2">
          <Input
            placeholder="Optional stash message..."
            value={stashMsg}
            onChange={(e) => setStashMsg(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && stashAction("save")}
            className="flex-1"
          />
          <Button
            size="sm"
            onClick={() => stashAction("save")}
            disabled={actionLoading === "save-new"}
          >
            Save
          </Button>
        </div>
      )}

      {stashes.length === 0 ? (
        <div className="rounded-lg border border-dashed p-6 text-center text-muted-foreground">
          <Package className="h-8 w-8 mx-auto mb-2 opacity-50" />
          No stashes
        </div>
      ) : (
        <div className="rounded-lg border divide-y">
          {stashes.map((stash) => {
            const idx = parseInt(stash.index.replace("stash@{", "").replace("}", ""));
            return (
              <div
                key={stash.index}
                className="flex items-center justify-between px-3 py-2 text-sm"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <Package className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                    <code className="text-xs text-primary">{stash.index}</code>
                    <span className="truncate">{stash.message}</span>
                  </div>
                  <div className="text-xs text-muted-foreground ml-5 mt-0.5">
                    {stash.date}
                  </div>
                </div>
                <div className="flex gap-1 shrink-0 ml-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2"
                    title="Apply"
                    onClick={() => stashAction("apply", idx)}
                    disabled={actionLoading === `apply-${idx}`}
                  >
                    <Copy className="h-3 w-3" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2"
                    title="Pop"
                    onClick={() => stashAction("pop", idx)}
                    disabled={actionLoading === `pop-${idx}`}
                  >
                    <Play className="h-3 w-3" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2 text-destructive hover:text-destructive"
                    title="Drop"
                    onClick={() => stashAction("drop", idx)}
                    disabled={actionLoading === `drop-${idx}`}
                  >
                    <Trash2 className="h-3 w-3" />
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
