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
    return (
      <div className="flex gap-3">
        <div className="w-8 h-8 rounded-full bg-yellow-500/10 flex items-center justify-center flex-shrink-0">
          <Bot className="h-4 w-4 text-yellow-600" />
        </div>
        <div className="max-w-[80%] rounded-lg p-2 bg-yellow-500/10 border border-yellow-500/20">
          <p className="text-xs font-medium text-yellow-700 dark:text-yellow-400 mb-1">
            {block.name || "Tool"}
          </p>
          <pre className="text-xs font-mono whitespace-pre-wrap text-muted-foreground">
            {block.output}
          </pre>
        </div>
      </div>
    );
  }

  // Skip permission blocks (handled separately by PermissionPrompt component)
  if (block.type === "permission") {
    return null;
  }

  // Assistant or thinking message
  if (block.type === "assistant" || block.type === "thinking") {
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
  }

  // Fallback for any other types
  return null;
});
