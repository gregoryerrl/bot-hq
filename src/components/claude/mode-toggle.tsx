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
