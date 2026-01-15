"use client";

import { useState, useRef, useEffect } from "react";
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
