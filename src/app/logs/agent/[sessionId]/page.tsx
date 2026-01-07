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
  const sessionId = parseInt(params.sessionId as string);
  const [session, setSession] = useState<SessionInfo | null>(null);

  useEffect(() => {
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
  }, [sessionId]);

  const title = session
    ? `${session.workspaceName} Agent`
    : `Agent #${sessionId}`;

  const subtitle = session?.taskTitle
    ? `Task: ${session.taskTitle}`
    : undefined;

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
