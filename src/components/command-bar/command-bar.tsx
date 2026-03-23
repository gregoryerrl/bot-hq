"use client";

import { useState, useRef, useEffect, useCallback, FormEvent, KeyboardEvent } from "react";
import { Loader2, Send, X, Command } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useCommandContext } from "./command-context";

export function CommandBar() {
  const { context, clearContext } = useCommandContext();
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [response, setResponse] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Cmd+K / Ctrl+K to focus
  useEffect(() => {
    function handleKeyDown(e: globalThis.KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        inputRef.current?.focus();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const handleSubmit = useCallback(
    async (e?: FormEvent) => {
      e?.preventDefault();
      const trimmed = input.trim();
      if (!trimmed || loading) return;

      setLoading(true);
      setError(null);
      setResponse(null);

      try {
        const res = await fetch("/api/command", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            input: trimmed,
            context: {
              projectId: context.projectId,
              diagramId: context.diagramId,
              taskId: context.taskId,
              label: context.label,
            },
          }),
        });

        if (!res.ok) {
          const data = await res.json().catch(() => ({ error: "Request failed" }));
          throw new Error(data.error || `Error ${res.status}`);
        }

        const data = await res.json();
        setResponse(data.result ?? data.message ?? JSON.stringify(data));
        setInput("");
      } catch (err) {
        setError(err instanceof Error ? err.message : "Something went wrong");
      } finally {
        setLoading(false);
      }
    },
    [input, loading, context]
  );

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit]
  );

  const placeholder = context.label
    ? `Ask about ${context.label}...`
    : "Ask anything...";

  return (
    <div className="border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      {/* Command bar */}
      <form onSubmit={handleSubmit} className="flex items-center gap-2 px-4 py-2">
        {context.label && (
          <Badge variant="secondary" className="shrink-0 gap-1">
            {context.label}
            <button
              type="button"
              onClick={clearContext}
              className="ml-0.5 rounded-full hover:bg-muted-foreground/20 p-0.5"
              aria-label="Clear context"
            >
              <X className="size-3" />
            </button>
          </Badge>
        )}

        <div className="relative flex-1">
          <input
            ref={inputRef}
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            disabled={loading}
            className="w-full rounded-md border border-input bg-transparent px-3 py-1.5 text-sm shadow-xs placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] disabled:pointer-events-none disabled:opacity-50"
          />
          <kbd className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 hidden h-5 select-none items-center gap-0.5 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground sm:flex">
            <Command className="size-2.5" />K
          </kbd>
        </div>

        <Button
          type="submit"
          size="icon-sm"
          variant="ghost"
          disabled={loading || !input.trim()}
          aria-label="Send"
        >
          {loading ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <Send className="size-4" />
          )}
        </Button>
      </form>

      {/* Response panel */}
      {(response || error) && (
        <div
          className={`mx-4 mb-2 rounded-md border px-3 py-2 text-sm ${
            error
              ? "border-destructive/50 bg-destructive/10 text-destructive"
              : "bg-muted/50 text-foreground"
          }`}
        >
          <div className="flex items-start justify-between gap-2">
            <p className="whitespace-pre-wrap flex-1">{error || response}</p>
            <button
              type="button"
              onClick={() => {
                setResponse(null);
                setError(null);
              }}
              className="shrink-0 rounded-full p-0.5 hover:bg-muted-foreground/20"
              aria-label="Dismiss"
            >
              <X className="size-3.5" />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
