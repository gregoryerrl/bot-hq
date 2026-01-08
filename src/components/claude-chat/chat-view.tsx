"use client";

import { useState, useEffect, useRef, memo, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Badge } from "@/components/ui/badge";
import {
  ArrowLeft,
  Send,
  Loader2,
  User,
  Bot,
  Folder,
} from "lucide-react";
import { cn } from "@/lib/utils";
import ReactMarkdown from "react-markdown";

interface Message {
  role: "user" | "assistant";
  content: string;
  timestamp: string;
}

// Memoized message component to prevent re-renders
const MessageBubble = memo(function MessageBubble({ message }: { message: Message }) {
  return (
    <div
      className={cn(
        "flex gap-3",
        message.role === "user" ? "justify-end" : "justify-start"
      )}
    >
      {message.role === "assistant" && (
        <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center flex-shrink-0">
          <Bot className="h-4 w-4" />
        </div>
      )}
      <div
        className={cn(
          "max-w-[80%] rounded-lg p-3",
          message.role === "user"
            ? "bg-primary text-primary-foreground"
            : "bg-muted"
        )}
      >
        {message.role === "assistant" ? (
          <div className="prose prose-sm dark:prose-invert max-w-none">
            <ReactMarkdown>{message.content}</ReactMarkdown>
          </div>
        ) : (
          <p className="text-sm whitespace-pre-wrap">
            {message.content}
          </p>
        )}
      </div>
      {message.role === "user" && (
        <div className="w-8 h-8 rounded-full bg-primary flex items-center justify-center flex-shrink-0">
          <User className="h-4 w-4 text-primary-foreground" />
        </div>
      )}
    </div>
  );
});

interface ChatViewProps {
  sessionId: string | null;
  projectPath: string;
  isNewSession?: boolean;
  onBack: () => void;
}

export function ChatView({
  sessionId,
  projectPath,
  isNewSession = false,
  onBack,
}: ChatViewProps) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [status, setStatus] = useState<"idle" | "loading_history" | "waiting" | "error">("idle");
  const [currentSessionId, setCurrentSessionId] = useState<string | null>(
    sessionId
  );
  const messagesEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  useEffect(() => {
    if (sessionId && !isNewSession) {
      fetchSessionHistory(sessionId);
    }
  }, [sessionId, isNewSession]);

  async function fetchSessionHistory(sid: string) {
    try {
      setStatus("loading_history");
      const res = await fetch(`/api/claude-chat/sessions/${sid}`);
      if (res.ok) {
        const data = await res.json();
        setMessages(data.messages || []);
      }
    } catch (error) {
      console.error("Failed to fetch session history:", error);
    } finally {
      setStatus("idle");
    }
  }

  const sendMessage = useCallback(async () => {
    if (!input.trim() || status === "waiting") return;

    const userMessage = input.trim();
    setInput("");

    // Add user message to display
    setMessages((prev) => [
      ...prev,
      {
        role: "user",
        content: userMessage,
        timestamp: new Date().toISOString(),
      },
    ]);

    setStatus("waiting");

    try {
      let response: string;

      if (currentSessionId) {
        // Resume existing session
        const res = await fetch("/api/claude-chat/message", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            sessionId: currentSessionId,
            message: userMessage,
            projectPath,
          }),
        });

        if (!res.ok) {
          const errorData = await res.json().catch(() => ({}));
          throw new Error(errorData.error || "Failed to send message");
        }

        const data = await res.json();
        response = data.response;
      } else {
        // New session
        const res = await fetch("/api/claude-chat/new", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            message: userMessage,
            projectPath,
          }),
        });

        if (!res.ok) {
          const errorData = await res.json().catch(() => ({}));
          throw new Error(errorData.error || "Failed to start new session");
        }

        const data = await res.json();
        response = data.response;
        if (data.sessionId) {
          setCurrentSessionId(data.sessionId);
        }
      }

      // Add assistant response
      setMessages((prev) => [
        ...prev,
        {
          role: "assistant",
          content: response,
          timestamp: new Date().toISOString(),
        },
      ]);
      setStatus("idle");
    } catch (error) {
      console.error("Failed to send message:", error);
      setStatus("error");
      // Add error message
      setMessages((prev) => [
        ...prev,
        {
          role: "assistant",
          content: `Error: ${error instanceof Error ? error.message : String(error)}`,
          timestamp: new Date().toISOString(),
        },
      ]);
    }
  }, [input, status, currentSessionId, projectPath]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  }, [sendMessage]);

  const projectName = projectPath.split("/").pop() || projectPath;

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="p-3 border-b flex items-center gap-3">
        <Button size="icon" variant="ghost" onClick={onBack}>
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <Bot className="h-4 w-4" />
            <span className="font-medium">Claude Code</span>
          </div>
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Folder className="h-3 w-3" />
            {projectName}
            {isNewSession && (
              <Badge variant="secondary" className="text-xs">
                New Session
              </Badge>
            )}
          </div>
        </div>
      </div>

      {/* Messages */}
      <ScrollArea className="flex-1 p-4">
        {messages.length === 0 && status !== "loading_history" ? (
          <div className="text-center text-muted-foreground py-8">
            <Bot className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p>Start a conversation with Claude Code</p>
            <p className="text-sm mt-2">
              Working in: {projectPath}
            </p>
            <p className="text-xs mt-4 max-w-md mx-auto">
              This spawns the actual Claude Code CLI. To resume an existing session,
              go back and select one from the list.
            </p>
          </div>
        ) : (
          <div className="space-y-4">
            {messages.map((message, i) => (
              <MessageBubble key={`${message.timestamp}-${i}`} message={message} />
            ))}
            {status === "waiting" && (
              <div className="flex gap-3">
                <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                  <Bot className="h-4 w-4" />
                </div>
                <div className="bg-muted rounded-lg p-3 flex items-center gap-2">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span className="text-sm text-muted-foreground">Waiting for Claude...</span>
                </div>
              </div>
            )}
            {status === "loading_history" && (
              <div className="flex justify-center py-4">
                <div className="flex items-center gap-2 text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span className="text-sm">Loading conversation history...</span>
                </div>
              </div>
            )}
            {status === "error" && (
              <div className="flex justify-center py-2">
                <span className="text-sm text-destructive">Something went wrong. Try again.</span>
              </div>
            )}
            <div ref={messagesEndRef} />
          </div>
        )}
      </ScrollArea>

      {/* Input */}
      <div className="p-4 border-t">
        <div className="flex gap-2">
          <Textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type a message..."
            className="min-h-[60px] resize-none"
            disabled={status === "waiting"}
          />
          <Button
            size="icon"
            className="h-[60px] w-[60px]"
            onClick={sendMessage}
            disabled={!input.trim() || status === "waiting"}
          >
            {status === "waiting" ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}
