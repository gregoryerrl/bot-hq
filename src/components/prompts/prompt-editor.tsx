"use client";

import { useState, useEffect, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { ImproveDialog } from "./improve-dialog";
import { Save, RotateCcw, Sparkles } from "lucide-react";
import { toast } from "sonner";

interface PromptData {
  id: number;
  slug: string;
  name: string;
  description: string | null;
  content: string;
  variables: string | null;
  isParametric: boolean;
}

interface PromptEditorProps {
  prompt: PromptData;
  onSaved: () => void;
}

export function PromptEditor({ prompt, onSaved }: PromptEditorProps) {
  const [content, setContent] = useState(prompt.content);
  const [isDirty, setIsDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [improveOpen, setImproveOpen] = useState(false);

  const variables: string[] = prompt.variables
    ? JSON.parse(prompt.variables)
    : [];

  useEffect(() => {
    setContent(prompt.content);
    setIsDirty(false);
  }, [prompt.slug, prompt.content]);

  const handleContentChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      setContent(e.target.value);
      setIsDirty(e.target.value !== prompt.content);
    },
    [prompt.content]
  );

  async function handleSave() {
    setSaving(true);
    try {
      const res = await fetch(`/api/prompts/${prompt.slug}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ content }),
      });
      if (!res.ok) throw new Error("Failed to save");
      setIsDirty(false);
      toast.success("Prompt saved");
      onSaved();
    } catch {
      toast.error("Failed to save prompt");
    } finally {
      setSaving(false);
    }
  }

  async function handleReset() {
    try {
      const res = await fetch(`/api/prompts/${prompt.slug}/reset`, {
        method: "POST",
      });
      if (!res.ok) throw new Error("Failed to reset");
      const data = await res.json();
      setContent(data.content);
      setIsDirty(false);
      toast.success("Prompt reset to default");
      onSaved();
    } catch {
      toast.error("Failed to reset prompt");
    }
  }

  function handleAcceptImprovement(newContent: string) {
    setContent(newContent);
    setIsDirty(newContent !== prompt.content);
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="p-4 border-b space-y-2">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-lg font-semibold flex items-center gap-2">
              {prompt.name}
              {isDirty && (
                <span className="h-2 w-2 rounded-full bg-orange-500 inline-block" />
              )}
            </h2>
            {prompt.description && (
              <p className="text-sm text-muted-foreground">
                {prompt.description}
              </p>
            )}
          </div>
        </div>
        {variables.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {variables.map((v) => (
              <Badge key={v} variant="secondary" className="text-xs font-mono">
                {`{{${v}}}`}
              </Badge>
            ))}
          </div>
        )}
      </div>

      {/* Editor */}
      <div className="flex-1 min-h-0 p-4">
        <textarea
          value={content}
          onChange={handleContentChange}
          className="w-full h-full resize-none rounded-md border bg-background px-3 py-2 text-sm font-mono focus:outline-none focus:ring-2 focus:ring-ring"
          spellCheck={false}
        />
      </div>

      {/* Actions */}
      <div className="p-4 border-t flex items-center gap-2">
        <Button onClick={handleSave} disabled={!isDirty || saving} size="sm">
          <Save className="h-4 w-4 mr-1.5" />
          {saving ? "Saving..." : "Save"}
        </Button>
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button variant="outline" size="sm">
              <RotateCcw className="h-4 w-4 mr-1.5" />
              Reset to Default
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Reset to default?</AlertDialogTitle>
              <AlertDialogDescription>
                This will overwrite your current content with the hardcoded
                default. This cannot be undone.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction onClick={handleReset}>
                Reset
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setImproveOpen(true)}
        >
          <Sparkles className="h-4 w-4 mr-1.5" />
          Improve
        </Button>

        <ImproveDialog
          open={improveOpen}
          onOpenChange={setImproveOpen}
          slug={prompt.slug}
          currentContent={content}
          onAccept={handleAcceptImprovement}
        />
      </div>
    </div>
  );
}

export function EmptyPromptEditor() {
  return (
    <div className="flex items-center justify-center h-full text-muted-foreground">
      Select a prompt to edit
    </div>
  );
}
