"use client";

import { useEffect, useState, useCallback, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Bot, Loader2, RefreshCw, Circle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ModeToggle } from "./mode-toggle";
import { TerminalView } from "./terminal-view";
import { ChatView } from "./chat-view";
import { useIsMobile } from "@/hooks/use-media-query";
import { detectPermissionPrompt, detectSelectionMenu, detectAwaitingInput, AwaitingInputPrompt } from "@/lib/terminal-parser";
import { useNotificationContext } from "@/components/notifications/notification-provider";
import "@xterm/xterm/css/xterm.css";

type ViewMode = "terminal" | "chat";
type SessionStatus = "idle" | "streaming" | "permission" | "input" | "selection" | "awaiting_input";

// Single manager session state
interface ManagerSession {
  terminal: Terminal;
  fitAddon: FitAddon;
  eventSource: EventSource | null;
  buffer: string;
}

export function ClaudeSession() {
  const [session, setSession] = useState<ManagerSession | null>(null);
  const [isConnecting, setIsConnecting] = useState(true);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [mode, setMode] = useState<ViewMode>("terminal");
  const [status, setStatus] = useState<SessionStatus>("idle");
  const isMobile = useIsMobile();
  const idleTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const lastAwaitingPromptRef = useRef<string | null>(null);
  const hasConnectedRef = useRef(false);
  const lastNotifiedStatusRef = useRef<SessionStatus | null>(null);
  const { addNotification } = useNotificationContext();

  // Notify when terminal needs input
  const updateStatusWithNotification = useCallback((newStatus: SessionStatus) => {
    setStatus(newStatus);

    // Only notify on status change and for input-requiring states
    if (newStatus !== lastNotifiedStatusRef.current) {
      if (newStatus === "permission") {
        addNotification("Terminal Needs Input", "Permission prompt is waiting for your response", "warning");
      } else if (newStatus === "awaiting_input") {
        addNotification("Terminal Needs Input", "Claude is asking a question", "warning");
      } else if (newStatus === "selection") {
        addNotification("Terminal Needs Input", "Selection menu is waiting", "warning");
      }
      lastNotifiedStatusRef.current = newStatus;
    }
  }, [addNotification]);

  // Update task state when awaiting input is detected
  const updateTaskAwaitingState = useCallback(async (prompt: AwaitingInputPrompt | null, taskId?: number) => {
    if (prompt) {
      const promptKey = `${prompt.taskId || taskId || 'unknown'}-${prompt.question}`;
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
      lastAwaitingPromptRef.current = null;
    }
  }, []);

  // Connect to the manager session
  const connectToManager = useCallback(async () => {
    if (hasConnectedRef.current && session) {
      return; // Already connected
    }

    setIsConnecting(true);
    setConnectionError(null);

    try {
      // Ensure manager session exists on server
      const managerRes = await fetch("/api/terminal/manager");
      if (!managerRes.ok) {
        throw new Error("Failed to initialize manager session");
      }
      const managerData = await managerRes.json();
      const sessionId = managerData.sessionId;

      if (!sessionId) {
        throw new Error("No session ID returned from manager");
      }

      console.log("[ClaudeSession] Connecting to manager session:", sessionId);

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
            terminal.reset();
            terminal.write(message.data);
            buffer = message.data;
            setSession(prev => prev ? { ...prev, buffer } : prev);

            const awaitingInput = detectAwaitingInput(buffer);
            const menu = detectSelectionMenu(buffer);
            const prompt = detectPermissionPrompt(buffer);
            if (awaitingInput) {
              updateStatusWithNotification("awaiting_input");
              updateTaskAwaitingState(awaitingInput);
            } else if (menu) {
              updateStatusWithNotification("selection");
            } else if (prompt) {
              updateStatusWithNotification("permission");
            } else {
              setStatus("idle");
              updateTaskAwaitingState(null);
            }
          } else if (message.type === "data") {
            terminal.write(message.data);
            buffer += message.data;
            setSession(prev => prev ? { ...prev, buffer } : prev);

            const awaitingInput = detectAwaitingInput(buffer);
            const menu = detectSelectionMenu(buffer);
            const prompt = detectPermissionPrompt(buffer);
            if (awaitingInput) {
              updateStatusWithNotification("awaiting_input");
              updateTaskAwaitingState(awaitingInput);
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else if (menu) {
              updateStatusWithNotification("selection");
              if (idleTimeoutRef.current) {
                clearTimeout(idleTimeoutRef.current);
                idleTimeoutRef.current = null;
              }
            } else if (prompt) {
              updateStatusWithNotification("permission");
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
            hasConnectedRef.current = false;
            setSession(null);
          }
        } catch (e) {
          console.error("Failed to parse SSE message:", e);
        }
      };

      eventSource.onerror = () => {
        terminal.write("\r\n\x1b[31m[Connection lost - will retry...]\x1b[0m\r\n");
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

      const newSession: ManagerSession = {
        terminal,
        fitAddon,
        eventSource,
        buffer: "",
      };

      setSession(newSession);
      hasConnectedRef.current = true;
      setStatus("idle");

      if (isMobile) {
        setMode("chat");
      }
    } catch (error) {
      console.error("Failed to connect to manager session:", error);
      setConnectionError(error instanceof Error ? error.message : "Connection failed");
    } finally {
      setIsConnecting(false);
    }
  }, [isMobile, session, updateStatusWithNotification, updateTaskAwaitingState]);

  // Auto-connect on mount
  useEffect(() => {
    connectToManager();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Force chat mode on mobile
  useEffect(() => {
    if (isMobile && mode !== "chat") {
      setMode("chat");
    }
  }, [isMobile, mode]);

  // Send input to PTY (for chat view)
  const sendInput = useCallback(
    (input: string) => {
      fetch("/api/terminal/manager", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ input }),
      }).catch(console.error);
      setStatus("streaming");
    },
    []
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
      fetch("/api/terminal/manager", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ input: key }),
      }).catch(console.error);
    },
    []
  );

  // Send initial resize when terminal view becomes active
  useEffect(() => {
    if (mode === "terminal" && session) {
      const { cols, rows } = session.terminal;
      fetch("/api/terminal/manager", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ resize: { cols, rows } }),
      }).catch(console.error);
    }
  }, [mode, session]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (idleTimeoutRef.current) {
        clearTimeout(idleTimeoutRef.current);
      }
      if (session) {
        session.eventSource?.close();
        session.terminal.dispose();
      }
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Get status indicator color
  const getStatusColor = () => {
    switch (status) {
      case "streaming":
        return "text-green-500";
      case "permission":
      case "awaiting_input":
      case "selection":
        return "text-yellow-500";
      default:
        return "text-muted-foreground";
    }
  };

  const getStatusText = () => {
    switch (status) {
      case "streaming":
        return "Working...";
      case "permission":
        return "Permission needed";
      case "awaiting_input":
        return "Input needed";
      case "selection":
        return "Selection needed";
      default:
        return "Idle";
    }
  };

  return (
    <div className="flex flex-col h-[calc(100vh-4rem)]">
      {/* Header with status and mode toggle */}
      <div className="flex items-center justify-between gap-2 p-2 border-b bg-background">
        <div className="flex items-center gap-2">
          <Bot className="h-5 w-5 text-primary" />
          <span className="font-medium">Manager</span>
          <div className="flex items-center gap-1.5 text-sm">
            <Circle className={`h-2 w-2 fill-current ${getStatusColor()}`} />
            <span className="text-muted-foreground">{getStatusText()}</span>
          </div>
        </div>
        {!isMobile && session && (
          <ModeToggle mode={mode} onChange={setMode} />
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-hidden">
        {!session ? (
          <div className="h-full flex flex-col items-center justify-center text-muted-foreground bg-[#1a1b26]">
            <Bot className="h-16 w-16 mb-4 opacity-50" />
            <h3 className="text-lg font-medium mb-2">Claude Code Manager</h3>
            {isConnecting ? (
              <div className="flex items-center gap-2 text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>Connecting to manager session...</span>
              </div>
            ) : connectionError ? (
              <div className="flex flex-col items-center gap-4">
                <p className="text-sm text-red-400">{connectionError}</p>
                <Button onClick={connectToManager} variant="outline">
                  <RefreshCw className="h-4 w-4 mr-2" />
                  Retry Connection
                </Button>
              </div>
            ) : (
              <p className="text-sm">
                {!isMobile && "Toggle between terminal and chat views anytime."}
              </p>
            )}
          </div>
        ) : (
          <>
            {/* Always render terminal view on desktop to preserve state */}
            {!isMobile && (
              <TerminalView
                terminal={session.terminal}
                fitAddon={session.fitAddon}
                isVisible={mode === "terminal"}
              />
            )}
            {/* Chat view */}
            {(mode === "chat" || isMobile) && (
              <ChatView
                buffer={session.buffer}
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
