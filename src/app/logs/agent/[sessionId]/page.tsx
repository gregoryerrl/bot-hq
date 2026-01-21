"use client";

import { useEffect, useState } from "react";
import { useParams } from "next/navigation";
import { Header } from "@/components/layout/header";
import { LogDetail } from "@/components/log-viewer/log-detail";

interface SessionInfo {
  id: number;
  workspaceName: string;
  taskTitle: string;
  status: string;
}

export default function AgentLogsPage() {
  const params = useParams();
  const rawSessionId = params.sessionId as string;
  const sessionId = parseInt(rawSessionId);
  const isValidId = !isNaN(sessionId) && sessionId > 0;
  const [session, setSession] = useState<SessionInfo | null>(null);

  useEffect(() => {
    if (!isValidId) return;

    async function fetchSession() {
      try {
        const response = await fetch("/api/agents/sessions");
        if (response.ok) {
          const sessions = await response.json();
          const found = sessions.find((s: SessionInfo) => s.id === sessionId);
          if (found) {
            setSession(found);
          }
        }
      } catch (error) {
        console.error("Failed to fetch session:", error);
      }
    }
    fetchSession();
  }, [sessionId, isValidId]);

  const title = session
    ? `${session.workspaceName} Agent`
    : isValidId
    ? `Task #${sessionId}`
    : "Invalid Task";

  const subtitle = session?.taskTitle
    ? `Task: ${session.taskTitle}`
    : undefined;

  if (!isValidId) {
    return (
      <div className="flex flex-col h-full">
        <Header
          title="Agent Logs"
          description="Invalid Task ID"
        />
        <div className="flex-1 p-4 md:p-6">
          <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
            Invalid task ID. Please select a valid task from the logs page.
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Agent Logs"
        description={title}
      />
      <div className="flex-1 p-4 md:p-6">
        <LogDetail
          title={title}
          source="agent"
          sessionId={sessionId}
          subtitle={subtitle}
        />
      </div>
    </div>
  );
}
