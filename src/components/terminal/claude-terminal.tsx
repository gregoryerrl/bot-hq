"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Plus, X, Loader2, Terminal as TerminalIcon, RefreshCw } from "lucide-react";
import "@xterm/xterm/css/xterm.css";

interface TerminalSession {
  id: string;
  terminal: Terminal;
  fitAddon: FitAddon;
  eventSource: EventSource | null;
}

export function ClaudeTerminal() {
  const [sessions, setSessions] = useState<TerminalSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const terminalContainerRef = useRef<HTMLDivElement>(null);

  const activeSession = sessions.find((s) => s.id === activeSessionId);

  // Create a new terminal session
  const createSession = useCallback(async () => {
    setIsCreating(true);
    try {
      const res = await fetch("/api/terminal", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });

      if (!res.ok) {
        throw new Error("Failed to create session");
      }

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

      // Connect to SSE stream
      const eventSource = new EventSource(`/api/terminal/${sessionId}/stream`);

      eventSource.onmessage = (event) => {
        try {
          const message = JSON.parse(event.data);
          if (message.type === "data") {
            terminal.write(message.data);
          } else if (message.type === "exit") {
            terminal.write(`\r\n\x1b[33m[Session ended with code ${message.exitCode}]\x1b[0m\r\n`);
            eventSource.close();
          }
        } catch (e) {
          console.error("Failed to parse SSE message:", e);
        }
      };

      eventSource.onerror = () => {
        terminal.write("\r\n\x1b[31m[Connection lost]\x1b[0m\r\n");
      };

      // Handle input
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

      const newSession: TerminalSession = {
        id: sessionId,
        terminal,
        fitAddon,
        eventSource,
      };

      setSessions((prev) => [...prev, newSession]);
      setActiveSessionId(sessionId);
    } catch (error) {
      console.error("Failed to create terminal session:", error);
    } finally {
      setIsCreating(false);
    }
  }, []);

  // Close a session
  const closeSession = useCallback(async (sessionId: string) => {
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
  }, [sessions, activeSessionId]);

  // Mount terminal when active session changes
  useEffect(() => {
    if (activeSession && terminalContainerRef.current) {
      // Clear container
      terminalContainerRef.current.innerHTML = "";

      // Open terminal in container
      activeSession.terminal.open(terminalContainerRef.current);
      activeSession.fitAddon.fit();

      // Send initial size
      const { cols, rows } = activeSession.terminal;
      fetch(`/api/terminal/${activeSession.id}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ resize: { cols, rows } }),
      }).catch(console.error);
    }
  }, [activeSession]);

  // Handle window resize
  useEffect(() => {
    const handleResize = () => {
      if (activeSession) {
        activeSession.fitAddon.fit();
      }
    };

    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [activeSession]);

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
      {/* Header with tabs */}
      <div className="flex items-center gap-2 p-2 border-b bg-background">
        <div className="flex items-center gap-1 flex-1 overflow-x-auto">
          {sessions.map((session) => (
            <div
              key={session.id}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-md cursor-pointer text-sm ${
                activeSessionId === session.id
                  ? "bg-primary text-primary-foreground"
                  : "bg-muted hover:bg-muted/80"
              }`}
              onClick={() => setActiveSessionId(session.id)}
            >
              <TerminalIcon className="h-3 w-3" />
              <span className="truncate max-w-[100px]">
                {session.id.slice(0, 8)}
              </span>
              <button
                className="ml-1 hover:text-destructive"
                onClick={(e) => {
                  e.stopPropagation();
                  closeSession(session.id);
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
          onClick={createSession}
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

      {/* Terminal area */}
      <div className="flex-1 bg-[#1a1b26] p-2">
        {sessions.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-muted-foreground">
            <TerminalIcon className="h-16 w-16 mb-4 opacity-50" />
            <h3 className="text-lg font-medium mb-2">Claude Code Terminal</h3>
            <p className="text-sm mb-4 text-center max-w-md">
              Launch an interactive Claude Code session. You can approve tools,
              answer questions, and interact just like in your terminal.
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
        ) : (
          <div
            ref={terminalContainerRef}
            className="h-full w-full"
            style={{ minHeight: "400px" }}
          />
        )}
      </div>
    </div>
  );
}
