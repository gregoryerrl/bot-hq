"use client";

import { useState, useEffect } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  MessageSquare,
  Clock,
  Folder,
  Plus,
  RefreshCw,
} from "lucide-react";
import { formatDistanceToNow } from "date-fns";

interface SessionInfo {
  sessionId: string;
  projectPath: string;
  projectName: string;
  firstMessage: string;
  lastActivityAt: string;
  messageCount: number;
}

interface SessionListProps {
  onSelectSession: (sessionId: string, projectPath: string) => void;
  onNewSession: () => void;
}

export function SessionList({ onSelectSession, onNewSession }: SessionListProps) {
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [scopePath, setScopePath] = useState<string>("");
  const [loading, setLoading] = useState(true);

  async function fetchSessions() {
    try {
      setLoading(true);
      const res = await fetch("/api/claude-chat/sessions");
      const data = await res.json();
      setSessions(data.sessions || []);
      setScopePath(data.scopePath || "");
    } catch (error) {
      console.error("Failed to fetch sessions:", error);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    fetchSessions();
  }, []);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="p-4 border-b flex items-center justify-between">
        <div>
          <h2 className="font-semibold">Claude Code Sessions</h2>
          <p className="text-xs text-muted-foreground">Scope: {scopePath}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button size="icon" variant="ghost" onClick={fetchSessions}>
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          </Button>
          <Button size="sm" onClick={onNewSession}>
            <Plus className="h-4 w-4 mr-1" />
            New
          </Button>
        </div>
      </div>

      {/* Session List */}
      <ScrollArea className="flex-1">
        {loading && sessions.length === 0 ? (
          <div className="p-4 text-center text-muted-foreground">
            Loading sessions...
          </div>
        ) : sessions.length === 0 ? (
          <div className="p-8 text-center text-muted-foreground">
            <MessageSquare className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p>No Claude Code sessions found</p>
            <p className="text-sm mt-2">
              Sessions from {scopePath} will appear here
            </p>
          </div>
        ) : (
          <div className="p-2 space-y-2">
            {sessions.map((session) => (
              <Card
                key={session.sessionId}
                className="p-3 cursor-pointer hover:bg-muted/50 transition-colors"
                onClick={() =>
                  onSelectSession(session.sessionId, session.projectPath)
                }
              >
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Badge variant="outline" className="text-xs">
                      <Folder className="h-3 w-3 mr-1" />
                      {session.projectName}
                    </Badge>
                    <span className="text-xs text-muted-foreground flex items-center gap-1">
                      <Clock className="h-3 w-3" />
                      {session.lastActivityAt
                        ? formatDistanceToNow(new Date(session.lastActivityAt), {
                            addSuffix: true,
                          })
                        : "Unknown"}
                    </span>
                  </div>
                  <p className="text-sm text-muted-foreground line-clamp-2">
                    {session.firstMessage}
                  </p>
                  <div className="text-xs text-muted-foreground">
                    {session.messageCount} messages
                  </div>
                </div>
              </Card>
            ))}
          </div>
        )}
      </ScrollArea>
    </div>
  );
}
