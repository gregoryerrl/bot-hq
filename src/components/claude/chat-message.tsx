"use client";

import { memo } from "react";
import { cn } from "@/lib/utils";
import type { ParsedBlock } from "@/lib/terminal-parser";

interface ChatMessageProps {
  block: ParsedBlock;
}

export const ChatMessage = memo(function ChatMessage({ block }: ChatMessageProps) {
  // Skip permission blocks (handled separately by PermissionPrompt component)
  if (block.type === "permission") {
    return null;
  }

  // User input - show with > prefix like terminal
  if (block.type === "user") {
    return (
      <div className="font-mono text-sm py-0.5">
        <span className="text-blue-500">❯ </span>
        <span className="text-foreground">{block.content}</span>
      </div>
    );
  }

  // Code block
  if (block.type === "code") {
    return (
      <pre className="font-mono text-sm py-1 pl-4 text-muted-foreground whitespace-pre-wrap overflow-x-auto">
        {block.content}
      </pre>
    );
  }

  // Tool output
  if (block.type === "tool") {
    return (
      <div className="font-mono text-sm py-0.5">
        <span className="text-yellow-500">● </span>
        <span className="text-yellow-600 dark:text-yellow-400">{block.name}</span>
        {block.output && (
          <pre className="pl-4 text-muted-foreground whitespace-pre-wrap mt-0.5">
            {block.output}
          </pre>
        )}
      </div>
    );
  }

  // Assistant message - clean terminal output
  if (block.type === "assistant" || block.type === "thinking") {
    return (
      <div className={cn(
        "font-mono text-sm py-0.5 whitespace-pre-wrap",
        block.type === "thinking" ? "text-muted-foreground italic" : "text-foreground"
      )}>
        <span className="text-green-500">● </span>
        {block.content}
      </div>
    );
  }

  // Fallback
  return null;
});
