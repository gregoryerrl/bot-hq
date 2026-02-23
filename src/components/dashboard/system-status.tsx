"use client";

import { useState } from "react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { RefreshCw } from "lucide-react";

interface SystemStatusProps {
  status: { running: boolean; sessionId: string | null };
}

export function SystemStatus({ status }: SystemStatusProps) {
  const [syncing, setSyncing] = useState(false);
  const [syncResult, setSyncResult] = useState<string | null>(null);

  async function handleStartup() {
    setSyncing(true);
    setSyncResult(null);
    try {
      const res = await fetch("/api/manager/startup", {
        method: "POST",
      });
      if (res.ok) {
        setSyncResult("Clearing session and re-injecting full startup prompt...");
        setTimeout(() => {
          setSyncing(false);
          setSyncResult(null);
        }, 60000);
        return;
      } else {
        const data = await res.json();
        setSyncResult(data.error || "Failed to restart");
      }
    } catch {
      setSyncResult("Failed to reach the Manager");
    }
    setSyncing(false);
  }

  return (
    <Card className="p-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div
            className={`h-3 w-3 rounded-full ${
              status.running ? "bg-green-500" : "bg-red-500"
            }`}
          />
          <div>
            <p className="text-sm font-medium">
              Manager {status.running ? "Running" : "Stopped"}
            </p>
            {status.sessionId && (
              <p className="text-xs text-muted-foreground">
                Session: {status.sessionId}
              </p>
            )}
          </div>
        </div>
        <div className="flex items-center gap-3">
          {syncResult && (
            <p className="text-xs text-muted-foreground">{syncResult}</p>
          )}
          <Button
            size="sm"
            variant="outline"
            onClick={handleStartup}
            disabled={syncing || !status.running}
          >
            <RefreshCw className={`h-4 w-4 mr-2 ${syncing ? "animate-spin" : ""}`} />
            {syncing ? "Running..." : "Run Startup Operations"}
          </Button>
        </div>
      </div>
    </Card>
  );
}
