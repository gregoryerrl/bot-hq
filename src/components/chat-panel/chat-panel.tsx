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
