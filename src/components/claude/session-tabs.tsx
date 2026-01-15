"use client";

import { Button } from "@/components/ui/button";
import { Plus, X, Loader2, Bot } from "lucide-react";

interface Session {
  id: string;
}

interface SessionTabsProps {
  sessions: Session[];
  activeSessionId: string | null;
  isCreating: boolean;
  onSelectSession: (id: string) => void;
  onCloseSession: (id: string) => void;
  onNewSession: () => void;
}

export function SessionTabs({
  sessions,
  activeSessionId,
  isCreating,
  onSelectSession,
  onCloseSession,
  onNewSession,
}: SessionTabsProps) {
  return (
    <div className="flex items-center gap-2 flex-1">
      <div className="flex items-center gap-1 flex-1 overflow-x-auto">
        {sessions.map((session) => (
          <div
            key={session.id}
            className={`flex items-center gap-1 px-3 py-1.5 rounded-md cursor-pointer text-sm ${
              activeSessionId === session.id
                ? "bg-primary text-primary-foreground"
                : "bg-muted hover:bg-muted/80"
            }`}
            onClick={() => onSelectSession(session.id)}
          >
            <Bot className="h-3 w-3" />
            <span className="truncate max-w-[100px]">
              {session.id.slice(0, 8)}
            </span>
            <button
              className="ml-1 hover:text-destructive"
              onClick={(e) => {
                e.stopPropagation();
                onCloseSession(session.id);
              }}
            >
              <X className="h-3 w-3" />
            </button>
          </div>
        ))}
      </div>

      <Button
        size="sm"
        variant="outline"
        onClick={onNewSession}
        disabled={isCreating}
      >
        {isCreating ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Plus className="h-4 w-4" />
        )}
        <span className="ml-1">New</span>
      </Button>
    </div>
  );
}
