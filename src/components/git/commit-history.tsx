"use client";

import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { RefreshCw, ChevronDown, ChevronRight } from "lucide-react";

interface Commit {
  hash: string;
  shortHash: string;
  message: string;
  author: string;
  date: string;
  refs: string;
}

interface FileDiff {
  path: string;
  additions: number;
  deletions: number;
  status: string;
  diff?: string;
}

interface CommitHistoryProps {
  workspaceId: string;
}

export function CommitHistory({ workspaceId }: CommitHistoryProps) {
  const [commits, setCommits] = useState<Commit[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [diffData, setDiffData] = useState<Record<string, FileDiff[]>>({});
  const [limit, setLimit] = useState(50);

  const fetchLog = useCallback(async () => {
    if (!workspaceId) return;
    setLoading(true);
    try {
      const res = await fetch(
        `/api/git/log?workspaceId=${workspaceId}&limit=${limit}`
      );
      if (res.ok) {
        const data = await res.json();
        setCommits(data.commits || []);
      }
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [workspaceId, limit]);

  useEffect(() => {
    fetchLog();
  }, [fetchLog]);

  async function toggleExpand(hash: string) {
    if (expanded === hash) {
      setExpanded(null);
      return;
    }
    setExpanded(hash);

    // Fetch diff if not cached
    if (!diffData[hash]) {
      try {
        // Use git show to get files changed in this commit
        const res = await fetch(
          `/api/git/status?workspaceId=${workspaceId}`
        );
        // We'll just show the commit info for now since we don't have a per-commit diff endpoint
        if (res.ok) {
          setDiffData((prev) => ({
            ...prev,
            [hash]: [],
          }));
        }
      } catch {
        // ignore
      }
    }
  }

  if (loading && commits.length === 0) {
    return <div className="text-sm text-muted-foreground p-4">Loading history...</div>;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium">Commit History</h4>
        <Button variant="ghost" size="sm" onClick={fetchLog}>
          <RefreshCw className="h-3.5 w-3.5" />
        </Button>
      </div>

      <div className="rounded-lg border divide-y">
        {commits.map((commit) => (
          <div key={commit.hash}>
            <button
              className="w-full flex items-start gap-2 px-3 py-2 text-sm text-left hover:bg-muted/50 transition-colors"
              onClick={() => toggleExpand(commit.hash)}
            >
              {expanded === commit.hash ? (
                <ChevronDown className="h-3.5 w-3.5 mt-0.5 shrink-0 text-muted-foreground" />
              ) : (
                <ChevronRight className="h-3.5 w-3.5 mt-0.5 shrink-0 text-muted-foreground" />
              )}
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 flex-wrap">
                  <code className="text-xs text-primary font-mono">
                    {commit.shortHash}
                  </code>
                  <span className="truncate">{commit.message}</span>
                  {commit.refs && (
                    <div className="flex gap-1 flex-wrap">
                      {commit.refs
                        .split(",")
                        .map((ref) => ref.trim())
                        .filter(Boolean)
                        .map((ref) => (
                          <Badge
                            key={ref}
                            variant="outline"
                            className="text-[10px] px-1"
                          >
                            {ref}
                          </Badge>
                        ))}
                    </div>
                  )}
                </div>
                <div className="text-xs text-muted-foreground mt-0.5">
                  {commit.author} - {commit.date}
                </div>
              </div>
            </button>
            {expanded === commit.hash && (
              <div className="px-3 pb-2 pl-8">
                <div className="text-xs font-mono text-muted-foreground bg-muted/50 rounded p-2">
                  {commit.hash}
                </div>
              </div>
            )}
          </div>
        ))}
        {commits.length === 0 && (
          <div className="px-3 py-4 text-sm text-muted-foreground text-center">
            No commits found
          </div>
        )}
      </div>

      {commits.length >= limit && (
        <div className="text-center">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setLimit((l) => l + 50)}
          >
            Load More
          </Button>
        </div>
      )}
    </div>
  );
}
