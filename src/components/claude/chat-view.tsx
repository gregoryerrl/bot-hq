"use client";

import { useState, useRef, useEffect } from "react";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { Send, Loader2, Bot } from "lucide-react";
import { ChatMessage } from "./chat-message";
import { PermissionPrompt } from "./permission-prompt";
import { SelectionMenu } from "./selection-menu";
import {
  parseTerminalOutput,
  detectPermissionPrompt,
  detectSelectionMenu,
  isTellClaudeSelected,
  type PermissionPrompt as PermissionPromptType,
} from "@/lib/terminal-parser";

interface ChatViewProps {
  buffer: string;
  onSendInput: (input: string) => void;
  onSelectOption: (index: number) => void;
  onSendKey: (key: string) => void;
  status: "idle" | "streaming" | "permission" | "input" | "selection" | "awaiting_input";
}

export function ChatView({
  buffer,
  onSendInput,
  onSelectOption,
  onSendKey,
  status,
}: ChatViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [input, setInput] = useState("");

  const blocks = parseTerminalOutput(buffer);
  const permissionPrompt = detectPermissionPrompt(buffer);
  const selectionMenu = detectSelectionMenu(buffer);
  const showInput = status === "idle" || status === "input" || status === "awaiting_input" ||
    (permissionPrompt && isTellClaudeSelected(permissionPrompt));
  const showPermissionButtons = status === "permission" && permissionPrompt && !isTellClaudeSelected(permissionPrompt);
  const showSelectionMenu = status === "selection" && selectionMenu;

  // Auto-scroll to bottom (instant, no animation for terminal feel)
  useEffect(() => {
    scrollRef.current?.scrollIntoView({ behavior: "instant" });
  }, [buffer]);

  // Global keyboard handler for Escape and arrow keys when menus are shown
  useEffect(() => {
    const handleGlobalKeyDown = (e: KeyboardEvent) => {
      // Handle Escape key for canceling menus/prompts
      if (e.key === "Escape") {
        if (showSelectionMenu || showPermissionButtons) {
          e.preventDefault();
          onSendKey("\x1b"); // Escape
        }
      }

      // Handle arrow keys when selection menu is shown
      if (showSelectionMenu && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        e.preventDefault();
        if (e.key === "ArrowUp") {
          onSendKey("\x1b[A"); // Up arrow
        } else {
          onSendKey("\x1b[B"); // Down arrow
        }
      }

      // Handle Enter key when selection menu is shown
      if (showSelectionMenu && e.key === "Enter") {
        e.preventDefault();
        onSendKey("\r"); // Enter
      }
    };

    window.addEventListener("keydown", handleGlobalKeyDown);
    return () => window.removeEventListener("keydown", handleGlobalKeyDown);
  }, [showSelectionMenu, showPermissionButtons, onSendKey]);

  const handleSend = () => {
    if (!input.trim()) return;
    const text = input.trim();
    setInput("");

    // Clear any autocomplete suggestion and existing input first
    onSendKey("\x1b");  // Escape - dismiss autocomplete popup
    setTimeout(() => {
      onSendKey("\x15");  // Ctrl+U - kill line
      setTimeout(() => {
        // Send text (paste)
        onSendInput(text);
        setTimeout(() => {
          // Dismiss any autocomplete that appeared over pasted text
          onSendKey("\x1b");  // Escape
          setTimeout(() => {
            onSendKey("\r");  // Enter
          }, 150);
        }, 200);
      }, 150);
    }, 100);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleOptionSelect = (index: number) => {
    // Navigate to the option using arrow keys, then press Enter
    // Claude Code permission prompts use arrow navigation, not number input
    const currentIndex = permissionPrompt?.selectedIndex ?? 0;
    const diff = index - currentIndex;

    if (diff > 0) {
      // Move down
      for (let i = 0; i < diff; i++) {
        onSendKey("\x1b[B"); // Down arrow
      }
    } else if (diff < 0) {
      // Move up
      for (let i = 0; i < Math.abs(diff); i++) {
        onSendKey("\x1b[A"); // Up arrow
      }
    }

    // Small delay then press Enter to select
    setTimeout(() => {
      onSendKey("\r"); // Enter
    }, 100);
  };

  const handleMenuSelect = (index: number) => {
    // Navigate to the item using arrow keys, then press Enter
    const currentIndex = selectionMenu?.selectedIndex ?? 0;
    const diff = index - currentIndex;

    if (diff > 0) {
      // Move down
      for (let i = 0; i < diff; i++) {
        onSendKey("\x1b[B"); // Down arrow
      }
    } else if (diff < 0) {
      // Move up
      for (let i = 0; i < Math.abs(diff); i++) {
        onSendKey("\x1b[A"); // Up arrow
      }
    }

    // Small delay then press Enter
    setTimeout(() => {
      onSendKey("\r"); // Enter
    }, 100);
  };

  const handleMenuCancel = () => {
    onSendKey("\x1b"); // Escape
  };

  return (
    <div className="flex flex-col h-full max-h-[100dvh]">
      {/* Native scrolling div for terminal feel - no animations */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 min-h-0">
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
          <div className="space-y-2">
            {blocks.map((block, i) => (
              <ChatMessage key={i} block={block} />
            ))}
            {status === "streaming" && (
              <div className="flex gap-2 items-center text-muted-foreground text-sm py-1">
                <Loader2 className="h-3 w-3 animate-spin" />
                <span>Claude is working...</span>
              </div>
            )}
            <div ref={scrollRef} />
          </div>
        )}
      </div>

      {/* Permission buttons */}
      {showPermissionButtons && permissionPrompt && (
        <PermissionPrompt
          prompt={permissionPrompt}
          onSelect={handleOptionSelect}
        />
      )}

      {/* Selection menu */}
      {showSelectionMenu && selectionMenu && (
        <div className="p-4 border-t pb-[max(1rem,env(safe-area-inset-bottom))] max-h-[50vh] overflow-y-auto">
          <SelectionMenu
            menu={selectionMenu}
            onSelect={handleMenuSelect}
            onCancel={handleMenuCancel}
          />
        </div>
      )}

      {/* Text input */}
      {showInput && (
        <div className="p-4 border-t pb-[max(1rem,env(safe-area-inset-bottom))]">
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
