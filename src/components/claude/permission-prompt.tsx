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
