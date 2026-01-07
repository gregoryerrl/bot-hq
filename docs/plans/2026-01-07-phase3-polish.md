# Phase 3: Polish Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Manager agent with chat interface, notifications system, and mobile-friendly responsive UI.

**Architecture:** Manager agent uses Anthropic API (Haiku) for orchestration tasks like summarizing work, bulk assignments, and answering questions. Chat panel is a collapsible side panel. Notifications use toast + optional browser notifications. Mobile UI uses responsive Tailwind breakpoints.

**Tech Stack:** Anthropic SDK, Server-Sent Events, Web Notifications API, Tailwind responsive utilities

---

## Task 1: Install Anthropic SDK

**Files:**
- Modify: `package.json`

**Step 1: Install Anthropic SDK**

Run:
```bash
npm install @anthropic-ai/sdk
```

**Step 2: Verify installation**

Run:
```bash
npm list @anthropic-ai/sdk
```

Expected: Shows installed version

**Step 3: Commit**

```bash
git add -A && git commit -m "chore: install Anthropic SDK"
```

---

## Task 2: Create Manager Agent Library

**Files:**
- Create: `src/lib/agents/manager.ts`
- Create: `src/lib/agents/manager-prompts.ts`

**Step 1: Create manager prompts**

Create `src/lib/agents/manager-prompts.ts`:

```typescript
export const MANAGER_SYSTEM_PROMPT = `You are a Manager Agent for Bot-HQ, a workflow automation system.

Your responsibilities:
- Summarize work across repositories
- Help prioritize and assign issues
- Answer questions about task status
- Provide insights on agent activity

You have access to:
- List of workspaces and their GitHub repos
- Tasks with their states (new, queued, analyzing, plan_ready, in_progress, pr_draft, done)
- Agent session status
- Recent logs

You do NOT:
- Write code or make commits
- Directly control repo agents
- Access external APIs

Be concise and helpful. Format responses for easy scanning.`;

export function buildContextPrompt(context: {
  workspaces: { name: string; githubRemote: string | null }[];
  taskCounts: Record<string, number>;
  recentLogs: { type: string; message: string; createdAt: Date }[];
  activeSessions: number;
}): string {
  return `Current Bot-HQ Status:

Workspaces: ${context.workspaces.length}
${context.workspaces.map(w => `- ${w.name}: ${w.githubRemote || 'No GitHub'}`).join('\n')}

Task Summary:
${Object.entries(context.taskCounts).map(([state, count]) => `- ${state}: ${count}`).join('\n')}

Active Agent Sessions: ${context.activeSessions}

Recent Activity:
${context.recentLogs.slice(0, 5).map(l => `- [${l.type}] ${l.message}`).join('\n')}`;
}
```

**Step 2: Create manager agent**

Create `src/lib/agents/manager.ts`:

```typescript
import Anthropic from "@anthropic-ai/sdk";
import { db, workspaces, tasks, logs, agentSessions } from "@/lib/db";
import { eq, desc, sql } from "drizzle-orm";
import { MANAGER_SYSTEM_PROMPT, buildContextPrompt } from "./manager-prompts";

const anthropic = new Anthropic({
  apiKey: process.env.ANTHROPIC_API_KEY,
});

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export async function getManagerContext() {
  const allWorkspaces = await db.select({
    name: workspaces.name,
    githubRemote: workspaces.githubRemote,
  }).from(workspaces);

  const taskCountsRaw = await db
    .select({
      state: tasks.state,
      count: sql<number>`count(*)`.as('count'),
    })
    .from(tasks)
    .groupBy(tasks.state);

  const taskCounts: Record<string, number> = {};
  for (const row of taskCountsRaw) {
    taskCounts[row.state] = row.count;
  }

  const recentLogs = await db
    .select()
    .from(logs)
    .orderBy(desc(logs.createdAt))
    .limit(10);

  const activeSessions = await db
    .select()
    .from(agentSessions)
    .where(eq(agentSessions.status, "running"));

  return {
    workspaces: allWorkspaces,
    taskCounts,
    recentLogs,
    activeSessions: activeSessions.length,
  };
}

export async function chatWithManager(
  messages: ChatMessage[],
  onChunk?: (chunk: string) => void
): Promise<string> {
  const context = await getManagerContext();
  const contextPrompt = buildContextPrompt(context);

  const response = await anthropic.messages.create({
    model: "claude-3-haiku-20240307",
    max_tokens: 1024,
    system: `${MANAGER_SYSTEM_PROMPT}\n\n${contextPrompt}`,
    messages: messages.map(m => ({
      role: m.role,
      content: m.content,
    })),
    stream: true,
  });

  let fullResponse = "";

  for await (const event of response) {
    if (event.type === "content_block_delta" && event.delta.type === "text_delta") {
      const text = event.delta.text;
      fullResponse += text;
      onChunk?.(text);
    }
  }

  return fullResponse;
}

export async function getQuickSummary(): Promise<string> {
  const context = await getManagerContext();

  const totalTasks = Object.values(context.taskCounts).reduce((a, b) => a + b, 0);
  const inProgress = (context.taskCounts["in_progress"] || 0) +
                     (context.taskCounts["analyzing"] || 0);
  const pendingReview = context.taskCounts["plan_ready"] || 0;
  const done = context.taskCounts["done"] || 0;

  return `${totalTasks} tasks | ${inProgress} in progress | ${pendingReview} pending review | ${done} done | ${context.activeSessions} agents active`;
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add manager agent library"
```

---

## Task 3: Create Manager Chat API

**Files:**
- Create: `src/app/api/manager/chat/route.ts`
- Create: `src/app/api/manager/summary/route.ts`

**Step 1: Create chat API with streaming**

Create `src/app/api/manager/chat/route.ts`:

```typescript
import { NextRequest } from "next/server";
import { chatWithManager, ChatMessage } from "@/lib/agents/manager";

export const dynamic = "force-dynamic";

export async function POST(request: NextRequest) {
  try {
    const { messages } = await request.json() as { messages: ChatMessage[] };

    if (!messages || !Array.isArray(messages)) {
      return new Response(
        JSON.stringify({ error: "messages array is required" }),
        { status: 400, headers: { "Content-Type": "application/json" } }
      );
    }

    const encoder = new TextEncoder();

    const stream = new ReadableStream({
      async start(controller) {
        try {
          await chatWithManager(messages, (chunk) => {
            controller.enqueue(encoder.encode(`data: ${JSON.stringify({ text: chunk })}\n\n`));
          });
          controller.enqueue(encoder.encode(`data: ${JSON.stringify({ done: true })}\n\n`));
          controller.close();
        } catch (error) {
          controller.enqueue(
            encoder.encode(`data: ${JSON.stringify({ error: "Chat failed" })}\n\n`)
          );
          controller.close();
        }
      },
    });

    return new Response(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  } catch (error) {
    console.error("Manager chat error:", error);
    return new Response(
      JSON.stringify({ error: "Failed to process chat" }),
      { status: 500, headers: { "Content-Type": "application/json" } }
    );
  }
}
```

**Step 2: Create summary API**

Create `src/app/api/manager/summary/route.ts`:

```typescript
import { NextResponse } from "next/server";
import { getQuickSummary } from "@/lib/agents/manager";

export async function GET() {
  try {
    const summary = await getQuickSummary();
    return NextResponse.json({ summary });
  } catch (error) {
    console.error("Summary error:", error);
    return NextResponse.json(
      { error: "Failed to get summary" },
      { status: 500 }
    );
  }
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add manager chat API with streaming"
```

---

## Task 4: Create Chat Panel Component

**Files:**
- Create: `src/components/chat-panel/chat-panel.tsx`
- Create: `src/components/chat-panel/chat-message.tsx`
- Create: `src/components/chat-panel/chat-input.tsx`
- Create: `src/hooks/use-manager-chat.ts`

**Step 1: Create chat hook**

Create `src/hooks/use-manager-chat.ts`:

```typescript
"use client";

import { useState, useCallback } from "react";

export interface Message {
  id: string;
  role: "user" | "assistant";
  content: string;
  isStreaming?: boolean;
}

export function useManagerChat() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isLoading, setIsLoading] = useState(false);

  const sendMessage = useCallback(async (content: string) => {
    const userMessage: Message = {
      id: Date.now().toString(),
      role: "user",
      content,
    };

    const assistantMessage: Message = {
      id: (Date.now() + 1).toString(),
      role: "assistant",
      content: "",
      isStreaming: true,
    };

    setMessages((prev) => [...prev, userMessage, assistantMessage]);
    setIsLoading(true);

    try {
      const chatHistory = [...messages, userMessage].map((m) => ({
        role: m.role,
        content: m.content,
      }));

      const response = await fetch("/api/manager/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ messages: chatHistory }),
      });

      const reader = response.body?.getReader();
      if (!reader) throw new Error("No reader");

      const decoder = new TextDecoder();
      let fullContent = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        const chunk = decoder.decode(value);
        const lines = chunk.split("\n");

        for (const line of lines) {
          if (line.startsWith("data: ")) {
            try {
              const data = JSON.parse(line.slice(6));
              if (data.text) {
                fullContent += data.text;
                setMessages((prev) =>
                  prev.map((m) =>
                    m.id === assistantMessage.id
                      ? { ...m, content: fullContent }
                      : m
                  )
                );
              }
              if (data.done) {
                setMessages((prev) =>
                  prev.map((m) =>
                    m.id === assistantMessage.id
                      ? { ...m, isStreaming: false }
                      : m
                  )
                );
              }
            } catch {
              // Ignore parse errors
            }
          }
        }
      }
    } catch (error) {
      console.error("Chat error:", error);
      setMessages((prev) =>
        prev.map((m) =>
          m.id === assistantMessage.id
            ? { ...m, content: "Sorry, something went wrong.", isStreaming: false }
            : m
        )
      );
    } finally {
      setIsLoading(false);
    }
  }, [messages]);

  const clearMessages = useCallback(() => {
    setMessages([]);
  }, []);

  return { messages, isLoading, sendMessage, clearMessages };
}
```

**Step 2: Create chat message component**

Create `src/components/chat-panel/chat-message.tsx`:

```tsx
import { cn } from "@/lib/utils";
import { Message } from "@/hooks/use-manager-chat";

interface ChatMessageProps {
  message: Message;
}

export function ChatMessage({ message }: ChatMessageProps) {
  const isUser = message.role === "user";

  return (
    <div
      className={cn(
        "flex gap-3 p-3 rounded-lg",
        isUser ? "bg-primary/10" : "bg-muted"
      )}
    >
      <div
        className={cn(
          "w-6 h-6 rounded-full flex items-center justify-center text-xs font-medium",
          isUser ? "bg-primary text-primary-foreground" : "bg-secondary"
        )}
      >
        {isUser ? "U" : "M"}
      </div>
      <div className="flex-1 space-y-1">
        <div className="text-xs text-muted-foreground">
          {isUser ? "You" : "Manager"}
        </div>
        <div className="text-sm whitespace-pre-wrap">
          {message.content}
          {message.isStreaming && (
            <span className="inline-block w-2 h-4 bg-foreground/50 animate-pulse ml-1" />
          )}
        </div>
      </div>
    </div>
  );
}
```

**Step 3: Create chat input component**

Create `src/components/chat-panel/chat-input.tsx`:

```tsx
"use client";

import { useState, KeyboardEvent } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Send } from "lucide-react";

interface ChatInputProps {
  onSend: (message: string) => void;
  disabled?: boolean;
}

export function ChatInput({ onSend, disabled }: ChatInputProps) {
  const [input, setInput] = useState("");

  function handleSend() {
    if (!input.trim() || disabled) return;
    onSend(input.trim());
    setInput("");
  }

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  return (
    <div className="flex gap-2 p-3 border-t">
      <Input
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Ask the manager..."
        disabled={disabled}
        className="flex-1"
      />
      <Button size="icon" onClick={handleSend} disabled={disabled || !input.trim()}>
        <Send className="h-4 w-4" />
      </Button>
    </div>
  );
}
```

**Step 4: Create chat panel component**

Create `src/components/chat-panel/chat-panel.tsx`:

```tsx
"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { MessageSquare, X, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { useManagerChat } from "@/hooks/use-manager-chat";
import { ChatMessage } from "./chat-message";
import { ChatInput } from "./chat-input";

export function ChatPanel() {
  const [isOpen, setIsOpen] = useState(false);
  const { messages, isLoading, sendMessage, clearMessages } = useManagerChat();

  return (
    <>
      {/* Toggle Button */}
      <Button
        size="icon"
        variant="secondary"
        className={cn(
          "fixed bottom-4 right-4 z-50 h-12 w-12 rounded-full shadow-lg",
          isOpen && "hidden"
        )}
        onClick={() => setIsOpen(true)}
      >
        <MessageSquare className="h-5 w-5" />
      </Button>

      {/* Chat Panel */}
      <div
        className={cn(
          "fixed right-0 top-0 h-full w-80 bg-background border-l shadow-xl z-50 flex flex-col transition-transform duration-200",
          isOpen ? "translate-x-0" : "translate-x-full"
        )}
      >
        {/* Header */}
        <div className="flex items-center justify-between p-3 border-b">
          <div className="flex items-center gap-2">
            <MessageSquare className="h-4 w-4" />
            <span className="font-medium">Manager</span>
          </div>
          <div className="flex items-center gap-1">
            <Button
              size="icon"
              variant="ghost"
              className="h-8 w-8"
              onClick={clearMessages}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
            <Button
              size="icon"
              variant="ghost"
              className="h-8 w-8"
              onClick={() => setIsOpen(false)}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        </div>

        {/* Messages */}
        <ScrollArea className="flex-1 p-3">
          {messages.length === 0 ? (
            <div className="text-center text-muted-foreground text-sm py-8">
              <p>Ask the manager about:</p>
              <ul className="mt-2 space-y-1">
                <li>&quot;Summarize today&apos;s work&quot;</li>
                <li>&quot;What tasks need attention?&quot;</li>
                <li>&quot;Status of repo X&quot;</li>
              </ul>
            </div>
          ) : (
            <div className="space-y-3">
              {messages.map((message) => (
                <ChatMessage key={message.id} message={message} />
              ))}
            </div>
          )}
        </ScrollArea>

        {/* Input */}
        <ChatInput onSend={sendMessage} disabled={isLoading} />
      </div>
    </>
  );
}
```

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: add chat panel component"
```

---

## Task 5: Add Chat Panel to Layout

**Files:**
- Modify: `src/app/layout.tsx`

**Step 1: Add ChatPanel to layout**

Modify `src/app/layout.tsx`:

```tsx
import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { Sidebar } from "@/components/layout/sidebar";
import { Toaster } from "@/components/ui/sonner";
import { ChatPanel } from "@/components/chat-panel/chat-panel";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Bot-HQ",
  description: "Workflow Automation System",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className={inter.className}>
        <div className="flex h-screen">
          <Sidebar />
          <main className="flex-1 overflow-auto">{children}</main>
        </div>
        <ChatPanel />
        <Toaster />
      </body>
    </html>
  );
}
```

**Step 2: Commit**

```bash
git add -A && git commit -m "feat: integrate chat panel into layout"
```

---

## Task 6: Create Notifications System

**Files:**
- Create: `src/lib/notifications/index.ts`
- Create: `src/hooks/use-notifications.ts`
- Create: `src/components/notifications/notification-provider.tsx`

**Step 1: Create notifications library**

Create `src/lib/notifications/index.ts`:

```typescript
export interface Notification {
  id: string;
  title: string;
  body: string;
  type: "info" | "success" | "warning" | "error";
  timestamp: Date;
  read: boolean;
}

export function requestNotificationPermission(): Promise<NotificationPermission> {
  if (!("Notification" in window)) {
    return Promise.resolve("denied" as NotificationPermission);
  }
  return Notification.requestPermission();
}

export function sendBrowserNotification(title: string, body: string): void {
  if (!("Notification" in window)) return;
  if (Notification.permission !== "granted") return;

  new Notification(title, {
    body,
    icon: "/favicon.ico",
  });
}
```

**Step 2: Create notifications hook**

Create `src/hooks/use-notifications.ts`:

```typescript
"use client";

import { useState, useEffect, useCallback } from "react";
import { toast } from "sonner";
import {
  Notification,
  requestNotificationPermission,
  sendBrowserNotification,
} from "@/lib/notifications";

export function useNotifications() {
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const [permission, setPermission] = useState<NotificationPermission>("default");

  useEffect(() => {
    if ("Notification" in window) {
      setPermission(Notification.permission);
    }
  }, []);

  const requestPermission = useCallback(async () => {
    const result = await requestNotificationPermission();
    setPermission(result);
    return result;
  }, []);

  const addNotification = useCallback(
    (
      title: string,
      body: string,
      type: Notification["type"] = "info"
    ) => {
      const notification: Notification = {
        id: Date.now().toString(),
        title,
        body,
        type,
        timestamp: new Date(),
        read: false,
      };

      setNotifications((prev) => [notification, ...prev].slice(0, 50));

      // Show toast
      const toastFn = type === "error" ? toast.error : type === "success" ? toast.success : toast;
      toastFn(title, { description: body });

      // Send browser notification if permitted
      if (permission === "granted" && document.hidden) {
        sendBrowserNotification(title, body);
      }
    },
    [permission]
  );

  const markAsRead = useCallback((id: string) => {
    setNotifications((prev) =>
      prev.map((n) => (n.id === id ? { ...n, read: true } : n))
    );
  }, []);

  const clearAll = useCallback(() => {
    setNotifications([]);
  }, []);

  const unreadCount = notifications.filter((n) => !n.read).length;

  return {
    notifications,
    unreadCount,
    permission,
    requestPermission,
    addNotification,
    markAsRead,
    clearAll,
  };
}
```

**Step 3: Create notification provider**

Create `src/components/notifications/notification-provider.tsx`:

```tsx
"use client";

import { createContext, useContext, useEffect, ReactNode } from "react";
import { useNotifications } from "@/hooks/use-notifications";

type NotificationContextType = ReturnType<typeof useNotifications>;

const NotificationContext = createContext<NotificationContextType | null>(null);

export function useNotificationContext() {
  const context = useContext(NotificationContext);
  if (!context) {
    throw new Error(
      "useNotificationContext must be used within NotificationProvider"
    );
  }
  return context;
}

interface NotificationProviderProps {
  children: ReactNode;
}

export function NotificationProvider({ children }: NotificationProviderProps) {
  const notifications = useNotifications();

  // Listen for log stream events that warrant notifications
  useEffect(() => {
    const eventSource = new EventSource("/api/logs/stream");

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.type === "connected") return;

        // Notify on important events
        if (data.type === "approval") {
          notifications.addNotification(
            "Approval Required",
            data.message,
            "warning"
          );
        } else if (data.type === "error") {
          notifications.addNotification("Error", data.message, "error");
        }
      } catch {
        // Ignore parse errors
      }
    };

    return () => eventSource.close();
  }, [notifications]);

  return (
    <NotificationContext.Provider value={notifications}>
      {children}
    </NotificationContext.Provider>
  );
}
```

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: add notifications system"
```

---

## Task 7: Add Notification Bell to Header

**Files:**
- Create: `src/components/notifications/notification-bell.tsx`
- Modify: `src/components/layout/header.tsx`
- Modify: `src/app/layout.tsx`

**Step 1: Create notification bell component**

Create `src/components/notifications/notification-bell.tsx`:

```tsx
"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Bell, Check, Trash2, Settings } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useNotificationContext } from "./notification-provider";
import { cn } from "@/lib/utils";

export function NotificationBell() {
  const {
    notifications,
    unreadCount,
    permission,
    requestPermission,
    markAsRead,
    clearAll,
  } = useNotificationContext();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" className="relative">
          <Bell className="h-4 w-4" />
          {unreadCount > 0 && (
            <Badge
              variant="destructive"
              className="absolute -top-1 -right-1 h-5 w-5 p-0 flex items-center justify-center text-xs"
            >
              {unreadCount > 9 ? "9+" : unreadCount}
            </Badge>
          )}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-80">
        <div className="flex items-center justify-between px-3 py-2">
          <span className="font-medium">Notifications</span>
          <div className="flex items-center gap-1">
            {permission !== "granted" && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 text-xs"
                onClick={requestPermission}
              >
                <Settings className="h-3 w-3 mr-1" />
                Enable
              </Button>
            )}
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={clearAll}
            >
              <Trash2 className="h-3 w-3" />
            </Button>
          </div>
        </div>
        <DropdownMenuSeparator />
        <ScrollArea className="h-64">
          {notifications.length === 0 ? (
            <div className="text-center text-muted-foreground text-sm py-8">
              No notifications
            </div>
          ) : (
            notifications.slice(0, 10).map((notification) => (
              <DropdownMenuItem
                key={notification.id}
                className={cn(
                  "flex items-start gap-2 p-3 cursor-pointer",
                  !notification.read && "bg-muted/50"
                )}
                onClick={() => markAsRead(notification.id)}
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-sm truncate">
                      {notification.title}
                    </span>
                    {!notification.read && (
                      <span className="w-2 h-2 rounded-full bg-primary flex-shrink-0" />
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground truncate">
                    {notification.body}
                  </p>
                  <p className="text-xs text-muted-foreground mt-1">
                    {new Date(notification.timestamp).toLocaleTimeString()}
                  </p>
                </div>
              </DropdownMenuItem>
            ))
          )}
        </ScrollArea>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
```

**Step 2: Update header to include notification bell**

Modify `src/components/layout/header.tsx`:

```tsx
import { NotificationBell } from "@/components/notifications/notification-bell";

interface HeaderProps {
  title: string;
  description?: string;
}

export function Header({ title, description }: HeaderProps) {
  return (
    <header className="border-b px-6 py-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold">{title}</h1>
          {description && (
            <p className="text-sm text-muted-foreground">{description}</p>
          )}
        </div>
        <NotificationBell />
      </div>
    </header>
  );
}
```

**Step 3: Wrap layout with NotificationProvider**

Update `src/app/layout.tsx`:

```tsx
import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { Sidebar } from "@/components/layout/sidebar";
import { Toaster } from "@/components/ui/sonner";
import { ChatPanel } from "@/components/chat-panel/chat-panel";
import { NotificationProvider } from "@/components/notifications/notification-provider";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Bot-HQ",
  description: "Workflow Automation System",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className={inter.className}>
        <NotificationProvider>
          <div className="flex h-screen">
            <Sidebar />
            <main className="flex-1 overflow-auto">{children}</main>
          </div>
          <ChatPanel />
          <Toaster />
        </NotificationProvider>
      </body>
    </html>
  );
}
```

**Step 4: Commit**

```bash
git add -A && git commit -m "feat: add notification bell to header"
```

---

## Task 8: Mobile-Responsive Sidebar

**Files:**
- Modify: `src/components/layout/sidebar.tsx`
- Create: `src/components/layout/mobile-nav.tsx`

**Step 1: Create mobile navigation**

Create `src/components/layout/mobile-nav.tsx`:

```tsx
"use client";

import { useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { Menu, X, LayoutDashboard, Clock, ScrollText, Settings } from "lucide-react";

const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/settings", label: "Settings", icon: Settings },
];

export function MobileNav() {
  const [isOpen, setIsOpen] = useState(false);
  const pathname = usePathname();

  return (
    <>
      {/* Mobile Menu Button - Only visible on small screens */}
      <div className="fixed top-4 left-4 z-50 md:hidden">
        <Button
          variant="outline"
          size="icon"
          onClick={() => setIsOpen(!isOpen)}
        >
          {isOpen ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
        </Button>
      </div>

      {/* Mobile Navigation Overlay */}
      {isOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden"
          onClick={() => setIsOpen(false)}
        />
      )}

      {/* Mobile Navigation Panel */}
      <div
        className={cn(
          "fixed left-0 top-0 h-full w-64 bg-background border-r z-50 transform transition-transform duration-200 md:hidden",
          isOpen ? "translate-x-0" : "-translate-x-full"
        )}
      >
        <div className="p-4 border-b mt-14">
          <h2 className="text-lg font-semibold">Bot-HQ</h2>
        </div>
        <nav className="p-4 space-y-2">
          {navItems.map((item) => {
            const Icon = item.icon;
            const isActive = pathname === item.href;

            return (
              <Link
                key={item.href}
                href={item.href}
                onClick={() => setIsOpen(false)}
                className={cn(
                  "flex items-center gap-3 px-3 py-2 rounded-md text-sm transition-colors",
                  isActive
                    ? "bg-primary text-primary-foreground"
                    : "hover:bg-muted"
                )}
              >
                <Icon className="h-4 w-4" />
                {item.label}
              </Link>
            );
          })}
        </nav>
      </div>
    </>
  );
}
```

**Step 2: Update sidebar for responsive behavior**

Modify `src/components/layout/sidebar.tsx`:

```tsx
"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { cn } from "@/lib/utils";
import { LayoutDashboard, Clock, ScrollText, Settings } from "lucide-react";

const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/settings", label: "Settings", icon: Settings },
];

export function Sidebar() {
  const pathname = usePathname();

  return (
    <aside className="hidden md:flex w-56 flex-col border-r bg-muted/30">
      <div className="p-4 border-b">
        <h2 className="text-lg font-semibold">Bot-HQ</h2>
      </div>
      <nav className="flex-1 p-4 space-y-2">
        {navItems.map((item) => {
          const Icon = item.icon;
          const isActive = pathname === item.href;

          return (
            <Link
              key={item.href}
              href={item.href}
              className={cn(
                "flex items-center gap-3 px-3 py-2 rounded-md text-sm transition-colors",
                isActive
                  ? "bg-primary text-primary-foreground"
                  : "hover:bg-muted"
              )}
            >
              <Icon className="h-4 w-4" />
              {item.label}
            </Link>
          );
        })}
      </nav>
    </aside>
  );
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add mobile-responsive sidebar"
```

---

## Task 9: Add Mobile Nav to Layout

**Files:**
- Modify: `src/app/layout.tsx`

**Step 1: Add MobileNav to layout**

Update `src/app/layout.tsx`:

```tsx
import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { Sidebar } from "@/components/layout/sidebar";
import { MobileNav } from "@/components/layout/mobile-nav";
import { Toaster } from "@/components/ui/sonner";
import { ChatPanel } from "@/components/chat-panel/chat-panel";
import { NotificationProvider } from "@/components/notifications/notification-provider";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Bot-HQ",
  description: "Workflow Automation System",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className={inter.className}>
        <NotificationProvider>
          <div className="flex h-screen">
            <Sidebar />
            <MobileNav />
            <main className="flex-1 overflow-auto md:ml-0 ml-0">{children}</main>
          </div>
          <ChatPanel />
          <Toaster />
        </NotificationProvider>
      </body>
    </html>
  );
}
```

**Step 2: Commit**

```bash
git add -A && git commit -m "feat: integrate mobile navigation"
```

---

## Task 10: Mobile-Responsive Page Layouts

**Files:**
- Modify: `src/app/page.tsx`
- Modify: `src/app/pending/page.tsx`
- Modify: `src/app/logs/page.tsx`
- Modify: `src/app/settings/page.tsx`
- Modify: `src/components/layout/header.tsx`

**Step 1: Update header for mobile**

Modify `src/components/layout/header.tsx`:

```tsx
import { NotificationBell } from "@/components/notifications/notification-bell";

interface HeaderProps {
  title: string;
  description?: string;
}

export function Header({ title, description }: HeaderProps) {
  return (
    <header className="border-b px-4 md:px-6 py-4">
      <div className="flex items-center justify-between">
        <div className="ml-10 md:ml-0">
          <h1 className="text-xl md:text-2xl font-semibold">{title}</h1>
          {description && (
            <p className="text-xs md:text-sm text-muted-foreground">{description}</p>
          )}
        </div>
        <NotificationBell />
      </div>
    </header>
  );
}
```

**Step 2: Update page layouts for mobile padding**

Modify `src/app/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";
import { TaskList } from "@/components/taskboard/task-list";
import { SyncButton } from "@/components/taskboard/sync-button";

export default function TaskboardPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Taskboard"
        description="Manage issues across all repositories"
      />
      <div className="flex-1 p-4 md:p-6">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-6">
          <div className="text-sm text-muted-foreground">
            Issues synced from GitHub
          </div>
          <SyncButton />
        </div>
        <TaskList />
      </div>
    </div>
  );
}
```

Modify `src/app/pending/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";
import { ApprovalList } from "@/components/pending-board/approval-list";

export default function PendingPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Pending Approvals"
        description="Actions waiting for your approval"
      />
      <div className="flex-1 p-4 md:p-6">
        <ApprovalList />
      </div>
    </div>
  );
}
```

Modify `src/app/logs/page.tsx`:

```tsx
import { Header } from "@/components/layout/header";
import { LogList } from "@/components/log-viewer/log-list";

export default function LogsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header title="Logs" description="Real-time activity stream" />
      <div className="flex-1 p-4 md:p-6">
        <LogList />
      </div>
    </div>
  );
}
```

Modify `src/app/settings/page.tsx`:

```tsx
"use client";

import { useState } from "react";
import { Header } from "@/components/layout/header";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { WorkspaceList } from "@/components/settings/workspace-list";
import { AddWorkspaceDialog } from "@/components/settings/add-workspace-dialog";
import { DeviceList } from "@/components/settings/device-list";
import { PairingDisplay } from "@/components/settings/pairing-display";

export default function SettingsPage() {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Settings"
        description="Configure workspaces and devices"
      />
      <div className="flex-1 p-4 md:p-6">
        <Tabs defaultValue="workspaces" className="space-y-6">
          <TabsList className="w-full sm:w-auto">
            <TabsTrigger value="workspaces" className="flex-1 sm:flex-initial">
              Workspaces
            </TabsTrigger>
            <TabsTrigger value="devices" className="flex-1 sm:flex-initial">
              Devices
            </TabsTrigger>
          </TabsList>

          <TabsContent value="workspaces" className="space-y-6">
            <WorkspaceList
              key={refreshKey}
              onAddClick={() => setDialogOpen(true)}
            />
          </TabsContent>

          <TabsContent value="devices" className="space-y-6">
            <PairingDisplay />
            <DeviceList />
          </TabsContent>
        </Tabs>
      </div>

      <AddWorkspaceDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onSuccess={() => setRefreshKey((k) => k + 1)}
      />
    </div>
  );
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add mobile-responsive page layouts"
```

---

## Task 11: Mobile-Responsive Cards

**Files:**
- Modify: `src/components/taskboard/task-card.tsx`
- Modify: `src/components/pending-board/approval-card.tsx`

**Step 1: Update task card for mobile**

Modify `src/components/taskboard/task-card.tsx`:

```tsx
"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Play, ExternalLink } from "lucide-react";
import { Task } from "@/lib/db/schema";

interface TaskCardProps {
  task: Task & { workspaceName?: string };
  onAssign: (taskId: number) => void;
  onStartAgent: (taskId: number) => void;
}

const stateColors: Record<string, string> = {
  new: "bg-gray-500",
  queued: "bg-yellow-500",
  analyzing: "bg-blue-500",
  plan_ready: "bg-purple-500",
  in_progress: "bg-orange-500",
  pr_draft: "bg-green-500",
  done: "bg-green-700",
};

const stateLabels: Record<string, string> = {
  new: "New",
  queued: "Queued",
  analyzing: "Analyzing",
  plan_ready: "Plan Ready",
  in_progress: "In Progress",
  pr_draft: "PR Draft",
  done: "Done",
};

export function TaskCard({ task, onAssign, onStartAgent }: TaskCardProps) {
  return (
    <Card className="p-3 md:p-4">
      <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex flex-wrap items-center gap-2 mb-1">
            {task.githubIssueNumber && (
              <span className="text-sm text-muted-foreground">
                #{task.githubIssueNumber}
              </span>
            )}
            <Badge
              variant="secondary"
              className={`${stateColors[task.state]} text-white text-xs`}
            >
              {stateLabels[task.state]}
            </Badge>
            {task.workspaceName && (
              <Badge variant="outline" className="text-xs">
                {task.workspaceName}
              </Badge>
            )}
          </div>
          <h3 className="font-medium text-sm md:text-base line-clamp-2">
            {task.title}
          </h3>
          {task.description && (
            <p className="text-xs md:text-sm text-muted-foreground mt-1 line-clamp-2">
              {task.description}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2 self-end sm:self-start">
          {task.state === "new" && (
            <Button size="sm" onClick={() => onAssign(task.id)}>
              Assign
            </Button>
          )}
          {task.state === "queued" && (
            <Button size="sm" onClick={() => onStartAgent(task.id)}>
              <Play className="h-4 w-4 mr-1" />
              Start
            </Button>
          )}
          {task.prUrl && (
            <Button size="sm" variant="outline" asChild>
              <a href={task.prUrl} target="_blank" rel="noopener noreferrer">
                <ExternalLink className="h-4 w-4" />
              </a>
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}
```

**Step 2: Update approval card for mobile**

Modify `src/components/pending-board/approval-card.tsx`:

```tsx
"use client";

import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Check, X, Terminal } from "lucide-react";
import { Approval } from "@/lib/db/schema";

interface ApprovalCardProps {
  approval: Approval & {
    taskTitle?: string;
    workspaceName?: string;
    githubIssueNumber?: number;
  };
  onApprove: (id: number) => void;
  onReject: (id: number) => void;
}

const typeLabels: Record<string, string> = {
  git_push: "Git Push",
  external_command: "External Command",
  deploy: "Deploy",
};

export function ApprovalCard({
  approval,
  onApprove,
  onReject,
}: ApprovalCardProps) {
  return (
    <Card className="p-3 md:p-4">
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary" className="text-xs">
            {typeLabels[approval.type]}
          </Badge>
          {approval.workspaceName && (
            <Badge variant="outline" className="text-xs">
              {approval.workspaceName}
            </Badge>
          )}
          {approval.githubIssueNumber && (
            <span className="text-xs text-muted-foreground">
              Issue #{approval.githubIssueNumber}
            </span>
          )}
        </div>

        {approval.taskTitle && (
          <h3 className="font-medium text-sm md:text-base">
            {approval.taskTitle}
          </h3>
        )}

        <div className="flex items-center gap-2 p-2 bg-muted rounded text-xs md:text-sm font-mono overflow-x-auto">
          <Terminal className="h-4 w-4 text-muted-foreground flex-shrink-0" />
          <code className="whitespace-nowrap">{approval.command}</code>
        </div>

        {approval.reason && (
          <p className="text-xs md:text-sm text-muted-foreground">
            {approval.reason}
          </p>
        )}

        <div className="flex items-center justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            className="text-green-600 hover:text-green-700 hover:bg-green-50"
            onClick={() => onApprove(approval.id)}
          >
            <Check className="h-4 w-4 mr-1" />
            <span className="hidden sm:inline">Approve</span>
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="text-red-600 hover:text-red-700 hover:bg-red-50"
            onClick={() => onReject(approval.id)}
          >
            <X className="h-4 w-4 mr-1" />
            <span className="hidden sm:inline">Reject</span>
          </Button>
        </div>
      </div>
    </Card>
  );
}
```

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: add mobile-responsive cards"
```

---

## Task 12: Final Verification

**Step 1: Build the project**

Run:
```bash
npm run build
```

Expected: Build succeeds with no errors

**Step 2: Run lint**

Run:
```bash
npm run lint
```

Expected: No lint errors

**Step 3: Push to GitHub**

```bash
git push
```

---

## Summary

Phase 3 creates:
- Manager agent library with Anthropic API integration
- Streaming chat API endpoint
- Chat panel component with real-time responses
- Notifications system with browser notifications
- Notification bell in header
- Mobile-responsive sidebar with hamburger menu
- Mobile-responsive page layouts
- Mobile-responsive card components

Total commits: ~12 focused commits
