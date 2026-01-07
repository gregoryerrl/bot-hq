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
