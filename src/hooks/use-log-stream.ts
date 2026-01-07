"use client";

import { useState, useEffect, useCallback } from "react";
import { Log } from "@/lib/db/schema";

interface UseLogStreamOptions {
  source?: "all" | "server" | "agent";
  sessionId?: number;
}

export function useLogStream(options: UseLogStreamOptions = {}) {
  const { source = "all", sessionId } = options;
  const [logs, setLogs] = useState<Log[]>([]);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    // Build URL with query params
    const params = new URLSearchParams();
    if (source !== "all") {
      params.set("source", source);
    }
    if (sessionId) {
      params.set("sessionId", sessionId.toString());
    }

    const url = `/api/logs/stream${params.toString() ? `?${params.toString()}` : ""}`;
    const eventSource = new EventSource(url);

    eventSource.onopen = () => {
      setConnected(true);
    };

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.type === "connected") {
          setConnected(true);
          return;
        }
        setLogs((prev) => [data, ...prev].slice(0, 200));
      } catch {
        // Ignore parse errors
      }
    };

    eventSource.onerror = () => {
      setConnected(false);
    };

    return () => {
      eventSource.close();
    };
  }, [source, sessionId]);

  const clearLogs = useCallback(() => {
    setLogs([]);
  }, []);

  return { logs, connected, clearLogs };
}
