# Unified Claude UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Merge Terminal and Claude Chat into a single "Claude" tab with real-time toggle between xterm.js and chat bubble views, both powered by the same PTY backend.

**Architecture:** Single PTY session streams output to both views via a shared buffer. Terminal view uses xterm.js directly. Chat view parses the buffer to extract messages, code blocks, and permission prompts rendered as clickable buttons. Mobile forces chat view.

**Tech Stack:** Next.js 16, React 19, xterm.js, node-pty, strip-ansi, Tailwind CSS, shadcn/ui components

---

## Task 1: Install strip-ansi dependency

**Files:**
- Modify: `package.json`

**Step 1: Install strip-ansi**

Run:
```bash
npm install strip-ansi
```

**Step 2: Verify installation**

Run:
```bash
grep strip-ansi package.json
```
Expected: `"strip-ansi": "^X.X.X"` in dependencies

**Step 3: Commit**

```bash
git add package.json package-lock.json
git commit -m "chore: add strip-ansi dependency for terminal parsing"
```

---

## Task 2: Create terminal parser utility

**Files:**
- Create: `src/lib/terminal-parser.ts`

**Step 1: Create the parser file**

```typescript
import stripAnsi from "strip-ansi";

export type ParsedBlock =
  | { type: "assistant"; content: string }
  | { type: "user"; content: string }
  | { type: "code"; content: string; lang?: string }
  | { type: "tool"; name: string; output: string }
  | { type: "permission"; question: string; options: string[] }
  | { type: "thinking"; content: string };

export interface PermissionPrompt {
  question: string;
  options: string[];
  selectedIndex: number;
}

// Detect if output ends with a permission prompt
export function detectPermissionPrompt(buffer: string): PermissionPrompt | null {
  const clean = stripAnsi(buffer);
  const lines = clean.split("\n").filter((l) => l.trim());

  // Look for pattern: ? question followed by numbered options with ❯ indicator
  // Find the last question line
  let questionIndex = -1;
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].trim().startsWith("?")) {
      questionIndex = i;
      break;
    }
  }

  if (questionIndex === -1) return null;

  const question = lines[questionIndex].replace(/^\?\s*/, "").trim();
  const options: string[] = [];
  let selectedIndex = 0;

  // Parse options after the question
  for (let i = questionIndex + 1; i < lines.length; i++) {
    const line = lines[i].trim();
    // Match patterns like "❯ 1. Yes" or "  2. No" or "> Yes"
    const optionMatch = line.match(/^([❯>]\s*)?(\d+\.\s*)?(.+)$/);
    if (optionMatch) {
      const isSelected = line.startsWith("❯") || line.startsWith(">");
      const optionText = optionMatch[3].trim();
      if (optionText && !optionText.startsWith("?")) {
        if (isSelected) selectedIndex = options.length;
        options.push(optionText);
      }
    }
  }

  if (options.length < 2) return null;

  return { question, options, selectedIndex };
}

// Parse full buffer into blocks for chat view
export function parseTerminalOutput(buffer: string): ParsedBlock[] {
  const clean = stripAnsi(buffer);
  const blocks: ParsedBlock[] = [];

  // Split by common delimiters
  const lines = clean.split("\n");
  let currentBlock: string[] = [];
  let currentType: ParsedBlock["type"] = "assistant";

  for (const line of lines) {
    // Detect user input (lines starting with > or after prompt)
    if (line.match(/^>\s/) || line.match(/^❯\s*\d+\./)) {
      // Flush current block
      if (currentBlock.length > 0) {
        blocks.push({ type: currentType, content: currentBlock.join("\n").trim() });
        currentBlock = [];
      }
      continue;
    }

    // Detect code blocks (lines with consistent indentation or ````)
    if (line.startsWith("```") || line.match(/^\s{4,}/)) {
      if (currentType !== "code" && currentBlock.length > 0) {
        blocks.push({ type: currentType, content: currentBlock.join("\n").trim() });
        currentBlock = [];
      }
      currentType = "code";
    }

    // Detect tool output (common patterns)
    if (line.match(/^(Read|Write|Edit|Bash|Glob|Grep):/i)) {
      if (currentBlock.length > 0) {
        blocks.push({ type: currentType, content: currentBlock.join("\n").trim() });
        currentBlock = [];
      }
      currentType = "tool";
    }

    currentBlock.push(line);
  }

  // Flush remaining
  if (currentBlock.length > 0) {
    const content = currentBlock.join("\n").trim();
    if (content) {
      blocks.push({ type: currentType, content });
    }
  }

  return blocks.filter((b) => b.type === "assistant" || b.type === "code" || b.type === "tool" ? b.content.length > 0 : true);
}

// Check if the "tell claude" option is selected
export function isTellClaudeSelected(prompt: PermissionPrompt): boolean {
  const selected = prompt.options[prompt.selectedIndex]?.toLowerCase() || "";
  return selected.includes("tell claude") || selected.includes("do differently") || selected.includes("feedback");
}
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/lib/terminal-parser.ts
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/lib/terminal-parser.ts
git commit -m "feat: add terminal output parser for chat view"
```

---

## Task 3: Create useMediaQuery hook

**Files:**
- Create: `src/hooks/use-media-query.ts`

**Step 1: Create the hook**

```typescript
"use client";

import { useState, useEffect } from "react";

export function useMediaQuery(query: string = "(max-width: 767px)"): boolean {
  const [matches, setMatches] = useState(false);

  useEffect(() => {
    const mediaQuery = window.matchMedia(query);
    setMatches(mediaQuery.matches);

    const handler = (event: MediaQueryListEvent) => {
      setMatches(event.matches);
    };

    mediaQuery.addEventListener("change", handler);
    return () => mediaQuery.removeEventListener("change", handler);
  }, [query]);

  return matches;
}

export function useIsMobile(): boolean {
  return useMediaQuery("(max-width: 767px)");
}
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/hooks/use-media-query.ts
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/hooks/use-media-query.ts
git commit -m "feat: add useMediaQuery and useIsMobile hooks"
```

---

## Task 4: Create session tabs component

**Files:**
- Create: `src/components/claude/session-tabs.tsx`

**Step 1: Create the component**

```typescript
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
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/components/claude/session-tabs.tsx
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/components/claude/session-tabs.tsx
git commit -m "feat: add SessionTabs component"
```

---

## Task 5: Create permission prompt component

**Files:**
- Create: `src/components/claude/permission-prompt.tsx`

**Step 1: Create the component**

```typescript
"use client";

import { Button } from "@/components/ui/button";
import type { PermissionPrompt as PermissionPromptType } from "@/lib/terminal-parser";

interface PermissionPromptProps {
  prompt: PermissionPromptType;
  onSelect: (index: number) => void;
  disabled?: boolean;
}

export function PermissionPrompt({
  prompt,
  onSelect,
  disabled = false,
}: PermissionPromptProps) {
  return (
    <div className="p-4 border-t bg-muted/30">
      <p className="text-sm font-medium mb-3">{prompt.question}</p>
      <div className="flex flex-wrap gap-2">
        {prompt.options.map((option, index) => (
          <Button
            key={index}
            variant={index === prompt.selectedIndex ? "default" : "outline"}
            size="sm"
            onClick={() => onSelect(index)}
            disabled={disabled}
            className="min-h-[44px]"
          >
            {option}
          </Button>
        ))}
      </div>
    </div>
  );
}
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/components/claude/permission-prompt.tsx
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/components/claude/permission-prompt.tsx
git commit -m "feat: add PermissionPrompt component"
```

---

## Task 6: Create chat message component

**Files:**
- Create: `src/components/claude/chat-message.tsx`

**Step 1: Create the component**

```typescript
"use client";

import { memo } from "react";
import { Bot, User } from "lucide-react";
import { cn } from "@/lib/utils";
import ReactMarkdown from "react-markdown";
import type { ParsedBlock } from "@/lib/terminal-parser";

interface ChatMessageProps {
  block: ParsedBlock;
}

export const ChatMessage = memo(function ChatMessage({ block }: ChatMessageProps) {
  if (block.type === "user") {
    return (
      <div className="flex gap-3 justify-end">
        <div className="max-w-[80%] rounded-lg p-3 bg-primary text-primary-foreground">
          <p className="text-sm whitespace-pre-wrap">{block.content}</p>
        </div>
        <div className="w-8 h-8 rounded-full bg-primary flex items-center justify-center flex-shrink-0">
          <User className="h-4 w-4 text-primary-foreground" />
        </div>
      </div>
    );
  }

  if (block.type === "code") {
    return (
      <div className="flex gap-3">
        <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center flex-shrink-0">
          <Bot className="h-4 w-4" />
        </div>
        <div className="max-w-[80%] rounded-lg p-3 bg-muted overflow-x-auto">
          <pre className="text-sm font-mono whitespace-pre-wrap">{block.content}</pre>
        </div>
      </div>
    );
  }

  if (block.type === "tool") {
    const toolBlock = block as { type: "tool"; name: string; output: string };
    return (
      <div className="flex gap-3">
        <div className="w-8 h-8 rounded-full bg-yellow-500/10 flex items-center justify-center flex-shrink-0">
          <Bot className="h-4 w-4 text-yellow-600" />
        </div>
        <div className="max-w-[80%] rounded-lg p-2 bg-yellow-500/10 border border-yellow-500/20">
          <p className="text-xs font-medium text-yellow-700 dark:text-yellow-400 mb-1">
            {toolBlock.name || "Tool"}
          </p>
          <pre className="text-xs font-mono whitespace-pre-wrap text-muted-foreground">
            {toolBlock.output || block.content}
          </pre>
        </div>
      </div>
    );
  }

  // Default: assistant message
  return (
    <div className="flex gap-3">
      <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center flex-shrink-0">
        <Bot className="h-4 w-4" />
      </div>
      <div className={cn("max-w-[80%] rounded-lg p-3 bg-muted")}>
        <div className="prose prose-sm dark:prose-invert max-w-none">
          <ReactMarkdown>{block.content}</ReactMarkdown>
        </div>
      </div>
    </div>
  );
});
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/components/claude/chat-message.tsx
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/components/claude/chat-message.tsx
git commit -m "feat: add ChatMessage component"
```

---

## Task 7: Create chat view component

**Files:**
- Create: `src/components/claude/chat-view.tsx`

**Step 1: Create the component**

```typescript
"use client";

import { useRef, useEffect } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { Send, Loader2, Bot } from "lucide-react";
import { ChatMessage } from "./chat-message";
import { PermissionPrompt } from "./permission-prompt";
import {
  parseTerminalOutput,
  detectPermissionPrompt,
  isTellClaudeSelected,
  type PermissionPrompt as PermissionPromptType,
} from "@/lib/terminal-parser";

interface ChatViewProps {
  buffer: string;
  onSendInput: (input: string) => void;
  onSelectOption: (index: number) => void;
  status: "idle" | "streaming" | "permission" | "input";
}

export function ChatView({
  buffer,
  onSendInput,
  onSelectOption,
  status,
}: ChatViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [input, setInput] = useState("");

  const blocks = parseTerminalOutput(buffer);
  const permissionPrompt = detectPermissionPrompt(buffer);
  const showInput = status === "idle" || status === "input" ||
    (permissionPrompt && isTellClaudeSelected(permissionPrompt));
  const showPermissionButtons = status === "permission" && permissionPrompt && !isTellClaudeSelected(permissionPrompt);

  // Auto-scroll to bottom
  useEffect(() => {
    scrollRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [buffer]);

  const handleSend = () => {
    if (!input.trim()) return;
    onSendInput(input.trim() + "\n");
    setInput("");
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleOptionSelect = (index: number) => {
    // Send the option number + Enter
    onSendInput(`${index + 1}\n`);
  };

  return (
    <div className="flex flex-col h-full">
      <ScrollArea className="flex-1 p-4">
        {blocks.length === 0 ? (
          <div className="text-center text-muted-foreground py-8">
            <Bot className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p>Start a conversation with Claude Code</p>
            <p className="text-xs mt-4 max-w-md mx-auto">
              This is a live Claude Code session. You can chat naturally or
              respond to prompts.
            </p>
          </div>
        ) : (
          <div className="space-y-4">
            {blocks.map((block, i) => (
              <ChatMessage key={i} block={block} />
            ))}
            {status === "streaming" && (
              <div className="flex gap-3">
                <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                  <Bot className="h-4 w-4" />
                </div>
                <div className="bg-muted rounded-lg p-3 flex items-center gap-2">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span className="text-sm text-muted-foreground">
                    Claude is working...
                  </span>
                </div>
              </div>
            )}
            <div ref={scrollRef} />
          </div>
        )}
      </ScrollArea>

      {/* Permission buttons */}
      {showPermissionButtons && permissionPrompt && (
        <PermissionPrompt
          prompt={permissionPrompt}
          onSelect={handleOptionSelect}
          disabled={status === "streaming"}
        />
      )}

      {/* Text input */}
      {showInput && (
        <div className="p-4 border-t">
          <div className="flex gap-2">
            <Textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Type a message..."
              className="min-h-[60px] resize-none"
              disabled={status === "streaming"}
            />
            <Button
              size="icon"
              className="h-[60px] w-[60px] min-h-[44px]"
              onClick={handleSend}
              disabled={!input.trim() || status === "streaming"}
            >
              {status === "streaming" ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Send className="h-4 w-4" />
              )}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

// Need to add useState import
import { useState } from "react";
```

**Step 2: Fix the import order (useState should be at top)**

The file has a bug - useState is imported at the bottom. Fix by ensuring imports are at top:

```typescript
"use client";

import { useState, useRef, useEffect } from "react";
// ... rest of imports
```

**Step 3: Verify file exists**

Run:
```bash
ls -la src/components/claude/chat-view.tsx
```
Expected: File exists

**Step 4: Commit**

```bash
git add src/components/claude/chat-view.tsx
git commit -m "feat: add ChatView component"
```

---

## Task 8: Create terminal view component

**Files:**
- Create: `src/components/claude/terminal-view.tsx`

**Step 1: Create the component**

```typescript
"use client";

import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface TerminalViewProps {
  terminal: Terminal | null;
  fitAddon: FitAddon | null;
}

export function TerminalView({ terminal, fitAddon }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (terminal && containerRef.current) {
      // Clear container
      containerRef.current.innerHTML = "";
      // Open terminal in container
      terminal.open(containerRef.current);
      fitAddon?.fit();
    }
  }, [terminal, fitAddon]);

  // Handle window resize
  useEffect(() => {
    const handleResize = () => {
      fitAddon?.fit();
    };

    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [fitAddon]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full bg-[#1a1b26]"
      style={{ minHeight: "400px" }}
    />
  );
}
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/components/claude/terminal-view.tsx
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/components/claude/terminal-view.tsx
git commit -m "feat: add TerminalView component"
```

---

## Task 9: Create mode toggle component

**Files:**
- Create: `src/components/claude/mode-toggle.tsx`

**Step 1: Create the component**

```typescript
"use client";

import { Terminal, MessageSquare } from "lucide-react";
import { cn } from "@/lib/utils";

interface ModeToggleProps {
  mode: "terminal" | "chat";
  onChange: (mode: "terminal" | "chat") => void;
  disabled?: boolean;
}

export function ModeToggle({ mode, onChange, disabled = false }: ModeToggleProps) {
  return (
    <div className="flex items-center bg-muted rounded-md p-0.5">
      <button
        className={cn(
          "flex items-center gap-1.5 px-3 py-1.5 rounded text-sm transition-colors",
          mode === "terminal"
            ? "bg-background text-foreground shadow-sm"
            : "text-muted-foreground hover:text-foreground"
        )}
        onClick={() => onChange("terminal")}
        disabled={disabled}
      >
        <Terminal className="h-3.5 w-3.5" />
        Terminal
      </button>
      <button
        className={cn(
          "flex items-center gap-1.5 px-3 py-1.5 rounded text-sm transition-colors",
          mode === "chat"
            ? "bg-background text-foreground shadow-sm"
            : "text-muted-foreground hover:text-foreground"
        )}
        onClick={() => onChange("chat")}
        disabled={disabled}
      >
        <MessageSquare className="h-3.5 w-3.5" />
        Chat
      </button>
    </div>
  );
}
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/components/claude/mode-toggle.tsx
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/components/claude/mode-toggle.tsx
git commit -m "feat: add ModeToggle component"
```

---

## Task 10: Create main ClaudeSession component

**Files:**
- Create: `src/components/claude/claude-session.tsx`

**Step 1: Create the component**

```typescript
"use client";

import { useEffect, useRef, useState, useCallback } from "react";
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
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/components/claude/claude-session.tsx
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/components/claude/claude-session.tsx
git commit -m "feat: add main ClaudeSession component with mode toggle"
```

---

## Task 11: Create the /claude page

**Files:**
- Create: `src/app/claude/page.tsx`

**Step 1: Create the page**

```typescript
import { ClaudeSession } from "@/components/claude/claude-session";

export default function ClaudePage() {
  return <ClaudeSession />;
}
```

**Step 2: Verify file exists**

Run:
```bash
ls -la src/app/claude/page.tsx
```
Expected: File exists

**Step 3: Commit**

```bash
git add src/app/claude/page.tsx
git commit -m "feat: add /claude page route"
```

---

## Task 12: Update sidebar navigation

**Files:**
- Modify: `src/components/layout/sidebar.tsx`

**Step 1: Update navItems to replace Terminal and Chat with single Claude item**

Change the navItems array from:
```typescript
const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/chat", label: "Claude Chat", icon: MessageSquare },
  { href: "/terminal", label: "Terminal", icon: Terminal },
  { href: "/docs", label: "Docs", icon: FileText },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/plugins", label: "Plugins", icon: Puzzle },
  { href: "/settings", label: "Settings", icon: Settings },
  { href: "/claude-settings", label: "Claude Settings", icon: Globe },
];
```

To:
```typescript
const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/claude", label: "Claude", icon: Bot },
  { href: "/docs", label: "Docs", icon: FileText },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/plugins", label: "Plugins", icon: Puzzle },
  { href: "/settings", label: "Settings", icon: Settings },
  { href: "/claude-settings", label: "Claude Settings", icon: Globe },
];
```

**Step 2: Add Bot import**

Add `Bot` to the lucide-react imports:
```typescript
import {
  LayoutDashboard,
  Clock,
  ScrollText,
  Settings,
  Globe,
  Bot,
  FileText,
  Puzzle,
  Box,
  LucideIcon,
} from "lucide-react";
```

Note: Remove `MessageSquare` and `Terminal` imports if they're no longer used elsewhere.

**Step 3: Verify changes**

Run:
```bash
grep -n "Claude" src/components/layout/sidebar.tsx
```
Expected: Shows the new Claude nav item

**Step 4: Commit**

```bash
git add src/components/layout/sidebar.tsx
git commit -m "feat: update sidebar with unified Claude nav item"
```

---

## Task 13: Update mobile navigation

**Files:**
- Modify: `src/components/layout/mobile-nav.tsx`

**Step 1: Update navItems to include Claude**

Change the navItems array to:
```typescript
const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/claude", label: "Claude", icon: Bot },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/settings", label: "Settings", icon: Settings },
  { href: "/claude-settings", label: "Claude Settings", icon: Globe },
];
```

**Step 2: Add Bot import**

```typescript
import { Menu, X, LayoutDashboard, Clock, ScrollText, Settings, Globe, Bot } from "lucide-react";
```

**Step 3: Verify changes**

Run:
```bash
grep -n "Claude" src/components/layout/mobile-nav.tsx
```
Expected: Shows the Claude nav item

**Step 4: Commit**

```bash
git add src/components/layout/mobile-nav.tsx
git commit -m "feat: update mobile nav with Claude item"
```

---

## Task 14: Remove old routes and components

**Files:**
- Delete: `src/app/chat/page.tsx`
- Delete: `src/app/terminal/page.tsx`
- Delete: `src/components/claude-chat/` (entire directory)
- Delete: `src/components/terminal/` (entire directory)

**Step 1: Remove old files**

Run:
```bash
rm -rf src/app/chat
rm -rf src/app/terminal
rm -rf src/components/claude-chat
rm -rf src/components/terminal
```

**Step 2: Verify removal**

Run:
```bash
ls src/app/chat 2>&1 || echo "chat removed"
ls src/app/terminal 2>&1 || echo "terminal removed"
ls src/components/claude-chat 2>&1 || echo "claude-chat removed"
ls src/components/terminal 2>&1 || echo "terminal component removed"
```
Expected: All show "removed" messages

**Step 3: Commit**

```bash
git add -A
git commit -m "refactor: remove old chat and terminal routes/components"
```

---

## Task 15: Fix ChatView import bug

**Files:**
- Modify: `src/components/claude/chat-view.tsx`

**Step 1: Fix the useState import**

The file has `useState` imported at the bottom. Move it to the top imports:

Change from:
```typescript
"use client";

import { useRef, useEffect } from "react";
// ... other imports

// At bottom of file:
import { useState } from "react";
```

To:
```typescript
"use client";

import { useState, useRef, useEffect } from "react";
// ... other imports
// Remove the duplicate import at bottom
```

**Step 2: Verify the build passes**

Run:
```bash
npm run build
```
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/components/claude/chat-view.tsx
git commit -m "fix: correct useState import in ChatView"
```

---

## Task 16: Test the application

**Step 1: Start the dev server**

Run:
```bash
npm run dev
```

**Step 2: Manual testing checklist**

- [ ] Navigate to `/claude` route
- [ ] Click "Start Session" to create a new session
- [ ] Verify terminal view works (on desktop)
- [ ] Toggle to chat view and verify it shows parsed output
- [ ] Toggle back to terminal view
- [ ] Test permission prompt buttons (if Claude asks for permission)
- [ ] Test text input in chat view
- [ ] Resize window to mobile size (<768px) and verify:
  - [ ] Mode toggle disappears
  - [ ] Only chat view is shown
- [ ] Create multiple sessions and switch between them
- [ ] Close sessions

**Step 3: Fix any issues found**

If issues are found, fix them and commit with appropriate message.

**Step 4: Final commit**

```bash
git add -A
git commit -m "test: verify unified Claude UI works correctly"
```

---

## Summary

This plan creates a unified Claude UI with:

1. **Single `/claude` route** replacing `/terminal` and `/chat`
2. **Real-time mode toggle** between Terminal (xterm.js) and Chat (bubbles) views
3. **Shared PTY session** - both views use the same buffer
4. **Permission prompts as buttons** in chat view
5. **Mobile-optimized** - chat view only, no toggle
6. **Clean removal** of old routes and components

Total: 16 tasks
