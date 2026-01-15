"use client";

import { useEffect, useState, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Bot, Loader2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SessionTabs } from "./session-tabs";
import { ModeToggle } from "./mode-toggle";
import { TerminalView } from "./terminal-view";
import { ChatView } from "./chat-view";
import { useIsMobile } from "@/hooks/use-media-query";
import { detectPermissionPrompt } from "@/lib/terminal-parser";
import "@xterm/xterm/css/xterm.css";

interface Session {
  id: string;
  terminal: Terminal;
  fitAddon: FitAddon;
  eventSource: EventSource | null;
  buffer: string;
}

type ViewMode = "terminal" | "chat";
type SessionStatus = "idle" | "streaming" | "permission" | "input";

export function ClaudeSession() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [mode, setMode] = useState<ViewMode>("terminal");
  const [status, setStatus] = useState<SessionStatus>("idle");
  const isMobile = useIsMobile();

  const activeSession = sessions.find((s) => s.id === activeSessionId);

  // Force chat mode on mobile
  useEffect(() => {
    if (isMobile && mode !== "chat") {
      setMode("chat");
    }
  }, [isMobile, mode]);

  // Create a new session
  const createSession = useCallback(async () => {
    setIsCreating(true);
    try {
      const res = await fetch("/api/terminal", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });

      if (!res.ok) throw new Error("Failed to create session");

      const { sessionId } = await res.json();

      // Create xterm instance
      const terminal = new Terminal({
        cursorBlink: true,
        fontSize: 14,
        fontFamily: 'Menlo, Monaco, "Courier New", monospace',
        theme: {
          background: "#1a1b26",
          foreground: "#a9b1d6",
          cursor: "#c0caf5",
          cursorAccent: "#1a1b26",
          selectionBackground: "#33467c",
          black: "#15161e",
          red: "#f7768e",
          green: "#9ece6a",
          yellow: "#e0af68",
          blue: "#7aa2f7",
          magenta: "#bb9af7",
          cyan: "#7dcfff",
          white: "#a9b1d6",
          brightBlack: "#414868",
          brightRed: "#f7768e",
          brightGreen: "#9ece6a",
          brightYellow: "#e0af68",
          brightBlue: "#7aa2f7",
          brightMagenta: "#bb9af7",
          brightCyan: "#7dcfff",
          brightWhite: "#c0caf5",
        },
      });

      const fitAddon = new FitAddon();
      terminal.loadAddon(fitAddon);

      let buffer = "";

      // Connect to SSE stream
      const eventSource = new EventSource(`/api/terminal/${sessionId}/stream`);

      eventSource.onmessage = (event) => {
        try {
          const message = JSON.parse(event.data);
          if (message.type === "data") {
            terminal.write(message.data);
            // Append to buffer for chat view
            buffer += message.data;
            setSessions((prev) =>
              prev.map((s) =>
                s.id === sessionId ? { ...s, buffer } : s
              )
            );
            // Check for permission prompt
            const prompt = detectPermissionPrompt(buffer);
            setStatus(prompt ? "permission" : "streaming");
          } else if (message.type === "exit") {
            terminal.write(
              `\r\n\x1b[33m[Session ended with code ${message.exitCode}]\x1b[0m\r\n`
            );
            eventSource.close();
            setStatus("idle");
          }
        } catch (e) {
          console.error("Failed to parse SSE message:", e);
        }
      };

      eventSource.onerror = () => {
        terminal.write("\r\n\x1b[31m[Connection lost]\x1b[0m\r\n");
        setStatus("idle");
      };

      // Handle terminal input
      terminal.onData((data) => {
        fetch(`/api/terminal/${sessionId}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ input: data }),
        }).catch(console.error);
      });

      // Handle resize
      terminal.onResize(({ cols, rows }) => {
        fetch(`/api/terminal/${sessionId}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ resize: { cols, rows } }),
        }).catch(console.error);
      });

      const newSession: Session = {
        id: sessionId,
        terminal,
        fitAddon,
        eventSource,
        buffer: "",
      };

      setSessions((prev) => [...prev, newSession]);
      setActiveSessionId(sessionId);
      setStatus("idle");

      // Set initial mode based on device
      if (isMobile) {
        setMode("chat");
      }
    } catch (error) {
      console.error("Failed to create session:", error);
    } finally {
      setIsCreating(false);
    }
  }, [isMobile]);

  // Close a session
  const closeSession = useCallback(
    async (sessionId: string) => {
      const session = sessions.find((s) => s.id === sessionId);
      if (session) {
        session.eventSource?.close();
        session.terminal.dispose();

        await fetch(`/api/terminal/${sessionId}`, {
          method: "DELETE",
        }).catch(console.error);

        setSessions((prev) => prev.filter((s) => s.id !== sessionId));

        if (activeSessionId === sessionId) {
          const remaining = sessions.filter((s) => s.id !== sessionId);
          setActiveSessionId(remaining.length > 0 ? remaining[0].id : null);
        }
      }
    },
    [sessions, activeSessionId]
  );

  // Send input to PTY (for chat view)
  const sendInput = useCallback(
    (input: string) => {
      if (!activeSessionId) return;
      fetch(`/api/terminal/${activeSessionId}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ input }),
      }).catch(console.error);
      setStatus("streaming");
    },
    [activeSessionId]
  );

  // Handle option selection (for permission prompts)
  const selectOption = useCallback(
    (index: number) => {
      sendInput(`${index + 1}\n`);
    },
    [sendInput]
  );

  // Send initial resize when terminal view becomes active
  useEffect(() => {
    if (mode === "terminal" && activeSession) {
      const { cols, rows } = activeSession.terminal;
      fetch(`/api/terminal/${activeSession.id}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ resize: { cols, rows } }),
      }).catch(console.error);
    }
  }, [mode, activeSession]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      sessions.forEach((session) => {
        session.eventSource?.close();
        session.terminal.dispose();
      });
    };
  }, []);

  return (
    <div className="flex flex-col h-[calc(100vh-4rem)]">
      {/* Header with tabs and mode toggle */}
      <div className="flex items-center justify-between gap-2 p-2 border-b bg-background">
        <SessionTabs
          sessions={sessions}
          activeSessionId={activeSessionId}
          isCreating={isCreating}
          onSelectSession={setActiveSessionId}
          onCloseSession={closeSession}
          onNewSession={createSession}
        />
        {!isMobile && activeSession && (
          <ModeToggle mode={mode} onChange={setMode} />
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-hidden">
        {sessions.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-muted-foreground bg-[#1a1b26]">
            <Bot className="h-16 w-16 mb-4 opacity-50" />
            <h3 className="text-lg font-medium mb-2">Claude Code</h3>
            <p className="text-sm mb-4 text-center max-w-md">
              Launch an interactive Claude Code session.
              {!isMobile && " Toggle between terminal and chat views anytime."}
            </p>
            <Button onClick={createSession} disabled={isCreating}>
              {isCreating ? (
                <Loader2 className="h-4 w-4 animate-spin mr-2" />
              ) : (
                <Plus className="h-4 w-4 mr-2" />
              )}
              Start Session
            </Button>
          </div>
        ) : mode === "terminal" && !isMobile ? (
          <TerminalView
            terminal={activeSession?.terminal ?? null}
            fitAddon={activeSession?.fitAddon ?? null}
          />
        ) : (
          <ChatView
            buffer={activeSession?.buffer ?? ""}
            onSendInput={sendInput}
            onSelectOption={selectOption}
            status={status}
          />
        )}
      </div>
    </div>
  );
}
