"use client";

import { cn } from "@/lib/utils";

interface PromptItem {
  id: number;
  slug: string;
  name: string;
  description: string | null;
  updatedAt: string | Date;
}

interface PromptListProps {
  prompts: PromptItem[];
  selectedSlug: string | null;
  onSelect: (slug: string) => void;
}

function formatRelativeTime(date: string | Date): string {
  const d = new Date(date);
  const now = new Date();
  const diffMs = now.getTime() - d.getTime();
  const diffMin = Math.floor(diffMs / 60000);
  const diffHr = Math.floor(diffMin / 60);
  const diffDay = Math.floor(diffHr / 24);

  if (diffMin < 1) return "just now";
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffHr < 24) return `${diffHr}h ago`;
  if (diffDay < 30) return `${diffDay}d ago`;
  return d.toLocaleDateString();
}

export function PromptList({ prompts, selectedSlug, onSelect }: PromptListProps) {
  return (
    <div className="space-y-1">
      {prompts.map((prompt) => (
        <button
          key={prompt.slug}
          onClick={() => onSelect(prompt.slug)}
          className={cn(
            "w-full text-left px-3 py-2.5 rounded-md transition-colors",
            selectedSlug === prompt.slug
              ? "bg-primary text-primary-foreground"
              : "hover:bg-muted"
          )}
        >
          <div className="text-sm font-medium">{prompt.name}</div>
          {prompt.description && (
            <div
              className={cn(
                "text-xs mt-0.5 line-clamp-1",
                selectedSlug === prompt.slug
                  ? "text-primary-foreground/70"
                  : "text-muted-foreground"
              )}
            >
              {prompt.description}
            </div>
          )}
          <div
            className={cn(
              "text-xs mt-1",
              selectedSlug === prompt.slug
                ? "text-primary-foreground/50"
                : "text-muted-foreground/70"
            )}
          >
            {formatRelativeTime(prompt.updatedAt)}
          </div>
        </button>
      ))}
    </div>
  );
}
