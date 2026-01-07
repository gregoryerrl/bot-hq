"use client";

import { useState, useEffect } from "react";
import { LogSourceCard } from "./log-source-card";

interface LogSource {
  id: string;
  type: "server" | "agent";
  name: string;
  status: "live" | "running";
  latestMessage: string | null;
  latestAt: string | null;
  sessionId?: number;
  taskTitle?: string;
  workspaceName?: string;
}

export function LogSourceList() {
  const [sources, setSources] = useState<LogSource[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    async function fetchSources() {
      try {
        const response = await fetch("/api/logs/sources");
        if (response.ok) {
          const data = await response.json();
          setSources(data);
        }
      } catch (error) {
        console.error("Failed to fetch log sources:", error);
      } finally {
        setLoading(false);
      }
    }

    fetchSources();

    // Poll every 3 seconds for updates
    const interval = setInterval(fetchSources, 3000);
    return () => clearInterval(interval);
  }, []);

  if (loading) {
    return (
      <div className="space-y-3">
        <div className="h-20 bg-muted/50 rounded-lg animate-pulse" />
        <div className="h-20 bg-muted/50 rounded-lg animate-pulse" />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {sources.map((source) => (
        <LogSourceCard key={source.id} source={source} />
      ))}

      {sources.length === 1 && sources[0].type === "server" && (
        <p className="text-sm text-muted-foreground text-center py-4">
          No active agents. Start an agent from the Taskboard.
        </p>
      )}
    </div>
  );
}
