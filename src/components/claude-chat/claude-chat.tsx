"use client";

import { useState, useEffect } from "react";
import { SessionList } from "./session-list";
import { ChatView } from "./chat-view";

interface SelectedSession {
  sessionId: string | null;
  projectPath: string;
  isNew: boolean;
}

export function ClaudeChat() {
  const [selectedSession, setSelectedSession] = useState<SelectedSession | null>(
    null
  );
  const [scopePath, setScopePath] = useState<string>("");

  // Fetch scope path on mount
  useEffect(() => {
    fetch("/api/settings?key=scope_path")
      .then((res) => res.json())
      .then((data) => setScopePath(data.value || ""))
      .catch(console.error);
  }, []);

  function handleSelectSession(sessionId: string, projectPath: string) {
    setSelectedSession({
      sessionId,
      projectPath,
      isNew: false,
    });
  }

  function handleNewSession() {
    setSelectedSession({
      sessionId: null,
      projectPath: scopePath,
      isNew: true,
    });
  }

  function handleBack() {
    setSelectedSession(null);
  }

  return (
    <div className="h-[calc(100vh-4rem)]">
      {selectedSession ? (
        <ChatView
          sessionId={selectedSession.sessionId}
          projectPath={selectedSession.projectPath}
          isNewSession={selectedSession.isNew}
          onBack={handleBack}
        />
      ) : (
        <SessionList
          onSelectSession={handleSelectSession}
          onNewSession={handleNewSession}
        />
      )}
    </div>
  );
}
