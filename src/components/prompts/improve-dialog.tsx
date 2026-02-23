"use client";

import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Loader2 } from "lucide-react";

interface ImproveDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  slug: string;
  currentContent: string;
  onAccept: (newContent: string) => void;
}

export function ImproveDialog({
  open,
  onOpenChange,
  slug,
  currentContent,
  onAccept,
}: ImproveDialogProps) {
  const [instruction, setInstruction] = useState("");
  const [loading, setLoading] = useState(false);
  const [suggestion, setSuggestion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<"current" | "suggested">("suggested");

  async function handleSubmit() {
    if (!instruction.trim()) return;
    setLoading(true);
    setError(null);
    setSuggestion(null);

    try {
      const res = await fetch(`/api/prompts/${slug}/improve`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ instruction }),
      });

      if (!res.ok) {
        const data = await res.json();
        throw new Error(data.error || "Failed to improve prompt");
      }

      const data = await res.json();
      setSuggestion(data.suggestion);
      setViewMode("suggested");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
    } finally {
      setLoading(false);
    }
  }

  function handleAccept() {
    if (suggestion) {
      onAccept(suggestion);
      handleClose();
    }
  }

  function handleClose() {
    setSuggestion(null);
    setInstruction("");
    setError(null);
    setViewMode("suggested");
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-3xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Improve Prompt</DialogTitle>
          <DialogDescription>
            Describe what you want to improve. An AI will suggest changes.
          </DialogDescription>
        </DialogHeader>

        {!suggestion ? (
          <div className="space-y-4">
            <Textarea
              placeholder="e.g., Make it more concise, add error handling instructions, improve the git workflow section..."
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              rows={4}
            />
            {error && (
              <div className="text-sm text-destructive">{error}</div>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={handleClose}>
                Cancel
              </Button>
              <Button
                onClick={handleSubmit}
                disabled={loading || !instruction.trim()}
              >
                {loading && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
                {loading ? "Generating..." : "Suggest Improvements"}
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex-1 flex flex-col min-h-0 space-y-4">
            <div className="flex gap-2">
              <Button
                variant={viewMode === "current" ? "default" : "outline"}
                size="sm"
                onClick={() => setViewMode("current")}
              >
                Current
              </Button>
              <Button
                variant={viewMode === "suggested" ? "default" : "outline"}
                size="sm"
                onClick={() => setViewMode("suggested")}
              >
                Suggested
              </Button>
            </div>
            <div className="flex-1 min-h-0 overflow-auto border rounded-md">
              <pre className="p-4 text-sm font-mono whitespace-pre-wrap">
                {viewMode === "current" ? currentContent : suggestion}
              </pre>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={handleClose}>
                Cancel
              </Button>
              <Button onClick={handleAccept}>Accept Suggestion</Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
