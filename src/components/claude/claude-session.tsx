"use client";

import { useEffect, useState, useCallback, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Bot, Loader2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SessionTabs } from "./session-tabs";
import { ModeToggle } from "./mode-toggle";
import { TerminalView } from "./terminal-view";
import { ChatView } from "./chat-view";
import { useIsMobile } from "@/hooks/use-media-query";
import { detectPermissionPrompt, detectSelectionMenu, detectAwaitingInput, AwaitingInputPrompt } from "@/lib/terminal-parser";
import "@xterm/xterm/css/xterm.css";

interface Session {
  id: string;
  terminal: Terminal;
  fitAddon: FitAddon;
  eventSource: EventSource | null;
  buffer: string;
}

type ViewMode = "terminal" | "chat";
type SessionStatus = "idle" | "streaming" | "permission" | "input" | "selection" | "awaiting_input";

interface ServerSession {
  id: string;
  createdAt: string;
  lastActivityAt: string;
  bufferSize: number;
}

export function ClaudeSession() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [serverSessions, setServerSessions] = useState<ServerSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [mode, setMode] = useState<ViewMode>("terminal");
  const [status, setStatus] = useState<SessionStatus>("idle");
  const isMobile = useIsMobile();
  const idleTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const lastAwaitingPromptRef = useRef<string | null>(null);

  // Update task state when awaiting input is detected
  const updateTaskAwaitingState = useCallback(async (prompt: AwaitingInputPrompt | null, taskId?: number) => {
    if (prompt) {
      const promptKey = `${prompt.taskId || taskId || 'unknown'}-${prompt.question}`;
      // Avoid duplicate API calls for the same prompt
      if (lastAwaitingPromptRef.current === promptKey) return;
      lastAwaitingPromptRef.current = promptKey;

      const targetTaskId = prompt.taskId || taskId;
      if (!targetTaskId) {
        console.log("[Session] Awaiting input detected but no taskId");
        return;
      }

      try {
        await fetch(`/api/tasks/${targetTaskId}`, {
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            state: "awaiting_input",
            waitingQuestion: prompt.question,
            waitingContext: prompt.options.length > 0
              ? `Options:\n${prompt.options.map((o, i) => `${i + 1}. ${o}`).join('\n')}`
              : null,
            waitingSince: new Date().toISOString(),
          }),
        });
        console.log("[Session] Task", targetTaskId, "set to awaiting_input");
      } catch (error) {
        console.error("[Session] Failed to update task state:", error);
      }
    } else if (lastAwaitingPromptRef.current) {
      // Prompt was cleared - but we don't auto-resume here
      // The manager will resume the task when it continues
      lastAwaitingPromptRef.current = null;
    }
  }, []);

  const activeSession = sessions.find((s) => s.id === activeSessionId);

  // Fetch existing server sessions and auto-connect to manager on mount
  useEffect(() => {
    const initializeManagerSession = async () => {
      try {
        // First, ensure manager session exists
        const managerRes = await fetch("/api/terminal/manager");
        if (managerRes.ok) {
          const managerData = await managerRes.json();
          console.log("[ClaudeSession] Manager session ready:", managerData.sessionId);

          // Fetch all sessions
          const res = await fetch("/api/terminal");
          if (res.ok) {
            const data = await res.json();
            setServerSessions(data.sessions || []);

            // Auto-connect to manager session if not already connected
            if (managerData.sessionId && !sessions.find(s => s.id === managerData.sessionId)) {
              console.log("[ClaudeSession] Auto-connecting to manager session...");
              connectToSession(managerData.sessionId);
            }
          }
        }
      } catch (error) {
        console.error("Failed to initialize manager session:", error);
      }
    };
    initializeManagerSession();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

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
          if (message.type === "buffer") {
            // Reconnection - reset terminal then replay buffered output
            // This prevents garbled display from stale cursor positioning
            terminal.reset();
            terminal.write(message.data);
            buffer = message.data;
            setSessions((prev) =>
              prev.map((s) =>
                s.id === sessionId ? { ...s, buffer } : s
              )
            );
            // Check status after buffer replay
            const menu = detectSelectionMenu(buffer);
            const prompt = detectPermissionPrompt(buffer);
            const awaitingInput = detectAwaitingInput(buffer);
            if (awaitingInput) {
              setStatus("awaiting_input");
              updateTaskAwaitingState(awaitingInput);
            } else if (menu) {
              setStatus("selection");
            } else if (prompt) {
              setStatus("permission");
            } else {
              setStatus("idle");
              updateTaskAwaitingState(null);
            }
          } else if (message.type === "data") {
            terminal.write(message.data);
            // Append to buffer for chat view
            buffer += message.data;
            setSessions((prev) =>
              prev.map((s) =>
                s.id === sessionId ? { ...s, buffer } : s
              )
            );
            // Check for awaiting input FIRST, then selection menu, then permission prompt
            const awaitingInput = detectAwaitingInput(buffer);
            const menu = detectSelectionMenu(buffer);
            const prompt = detectPermissionPrompt(buffer);
            if (awaitingInput) {
              setStatus("awaiting_input");
              updateTaskAwaitingState(awaitingInput);
              // Clear idle timeout when awaiting input detected
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else if (menu) {
              setStatus("selection");
              // Clear idle timeout when selection menu detected
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else if (prompt) {
              setStatus("permission");
              // Clear idle timeout when permission prompt detected
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else {
              setStatus("streaming");
              updateTaskAwaitingState(null);
              // Reset idle timeout - if no data for 1.5s, assume idle
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
              }
              idleTimeoutRef.current = setTimeout(() => {
                setStatus("idle");
              }, 1500);
            }
          } else if (message.type === "exit") {
            terminal.write(
              `\r\n\x1b[33m[Session ended with code ${message.exitCode}]\x1b[0m\r\n`
            );
            eventSource.close();
            if (idleTimeoutRef.current) {
              clearTimeout(idleTimeoutRef.current);
            }
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
      // Add to server sessions list
      setServerSessions((prev) => [
        ...prev,
        { id: sessionId, createdAt: new Date().toISOString(), lastActivityAt: new Date().toISOString(), bufferSize: 0 }
      ]);
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

  // Connect to an existing session (for persistence across devices)
  const connectToSession = useCallback(async (sessionId: string) => {
    // Check if already connected locally
    if (sessions.find(s => s.id === sessionId)) {
      setActiveSessionId(sessionId);
      return;
    }

    setIsCreating(true);
    try {
      // Create xterm instance for the existing session
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

      // Connect to SSE stream - will receive buffered output first
      const eventSource = new EventSource(`/api/terminal/${sessionId}/stream`);

      eventSource.onmessage = (event) => {
        try {
          const message = JSON.parse(event.data);
          if (message.type === "buffer") {
            // Reconnection - reset terminal then replay buffered output
            // This prevents garbled display from stale cursor positioning
            terminal.reset();
            terminal.write(message.data);
            buffer = message.data;
            setSessions((prev) =>
              prev.map((s) =>
                s.id === sessionId ? { ...s, buffer } : s
              )
            );
            const awaitingInput = detectAwaitingInput(buffer);
            const menu = detectSelectionMenu(buffer);
            const prompt = detectPermissionPrompt(buffer);
            if (awaitingInput) {
              setStatus("awaiting_input");
              updateTaskAwaitingState(awaitingInput);
            } else if (menu) {
              setStatus("selection");
            } else if (prompt) {
              setStatus("permission");
            } else {
              setStatus("idle");
              updateTaskAwaitingState(null);
            }
          } else if (message.type === "data") {
            terminal.write(message.data);
            buffer += message.data;
            setSessions((prev) =>
              prev.map((s) =>
                s.id === sessionId ? { ...s, buffer } : s
              )
            );
            const awaitingInput = detectAwaitingInput(buffer);
            const menu = detectSelectionMenu(buffer);
            const prompt = detectPermissionPrompt(buffer);
            if (awaitingInput) {
              setStatus("awaiting_input");
              updateTaskAwaitingState(awaitingInput);
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else if (menu) {
              setStatus("selection");
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else if (prompt) {
              setStatus("permission");
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else {
              setStatus("streaming");
              updateTaskAwaitingState(null);
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
              }
              idleTimeoutRef.current = setTimeout(() => {
                setStatus("idle");
              }, 1500);
            }
          } else if (message.type === "exit") {
            terminal.write(
              `\r\n\x1b[33m[Session ended with code ${message.exitCode}]\x1b[0m\r\n`
            );
            eventSource.close();
            if (idleTimeoutRef.current) {
              clearTimeout(idleTimeoutRef.current);
            }
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

      if (isMobile) {
        setMode("chat");
      }
    } catch (error) {
      console.error("Failed to connect to session:", error);
    } finally {
      setIsCreating(false);
    }
  }, [sessions, isMobile]);

  // Close a session (kills it on server)
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
        // Also remove from server sessions since it's killed
        setServerSessions((prev) => prev.filter((s) => s.id !== sessionId));

        if (activeSessionId === sessionId) {
          const remaining = sessions.filter((s) => s.id !== sessionId);
          setActiveSessionId(remaining.length > 0 ? remaining[0].id : null);
        }
      }
    },
    [sessions, activeSessionId]
  );

  // Disconnect from a session locally (keeps it running on server)
  const disconnectSession = useCallback(
    (sessionId: string) => {
      const session = sessions.find((s) => s.id === sessionId);
      if (session) {
        session.eventSource?.close();
        session.terminal.dispose();

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

  // Send raw key to PTY (for navigation in selection menus)
  const sendKey = useCallback(
    (key: string) => {
      if (!activeSessionId) return;
      fetch(`/api/terminal/${activeSessionId}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ input: key }),
      }).catch(console.error);
    },
    [activeSessionId]
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

  // Cleanup on unmount - just disconnect, don't kill sessions
  useEffect(() => {
    return () => {
      if (idleTimeoutRef.current) {
        clearTimeout(idleTimeoutRef.current);
      }
      // Only close EventSource and dispose terminal - sessions persist on server
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
          serverSessions={serverSessions}
          activeSessionId={activeSessionId}
          isCreating={isCreating}
          onSelectSession={setActiveSessionId}
          onCloseSession={closeSession}
          onNewSession={createSession}
          onConnectSession={connectToSession}
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
              {isCreating
                ? "Connecting to manager session..."
                : "Manager terminal session."}
              {!isMobile && !isCreating && " Toggle between terminal and chat views anytime."}
            </p>
            {isCreating ? (
              <div className="flex items-center gap-2 text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>Connecting...</span>
              </div>
            ) : (
              <Button onClick={async () => {
                // Try to reconnect to manager session
                setIsCreating(true);
                try {
                  const res = await fetch("/api/terminal/manager");
                  if (res.ok) {
                    const data = await res.json();
                    if (data.sessionId) {
                      connectToSession(data.sessionId);
                    }
                  }
                } catch (error) {
                  console.error("Failed to connect to manager:", error);
                } finally {
                  setIsCreating(false);
                }
              }}>
                <Plus className="h-4 w-4 mr-2" />
                Reconnect to Manager
              </Button>
            )}
          </div>
        ) : (
          <>
            {/* Always render terminal view on desktop to preserve state */}
            {!isMobile && (
              <TerminalView
                terminal={activeSession?.terminal ?? null}
                fitAddon={activeSession?.fitAddon ?? null}
                isVisible={mode === "terminal"}
              />
            )}
            {/* Chat view */}
            {(mode === "chat" || isMobile) && (
              <ChatView
                buffer={activeSession?.buffer ?? ""}
                onSendInput={sendInput}
                onSelectOption={selectOption}
                onSendKey={sendKey}
                status={status}
              />
            )}
          </>
        )}
      </div>
    </div>
  );
}
