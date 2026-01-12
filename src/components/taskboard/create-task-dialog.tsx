"use client";

import { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Loader2 } from "lucide-react";

interface Workspace {
  id: number;
  name: string;
  repoPath: string;
}

interface CreateTaskDialogProps {
  open: boolean;
  onClose: () => void;
  onTaskCreated: () => void;
}

export function CreateTaskDialog({
  open,
  onClose,
  onTaskCreated,
}: CreateTaskDialogProps) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [selectedWorkspace, setSelectedWorkspace] = useState<string>("");
  const [prompt, setPrompt] = useState("");
  const [refinedTitle, setRefinedTitle] = useState("");
  const [refinedDescription, setRefinedDescription] = useState("");
  const [loading, setLoading] = useState(false);
  const [refining, setRefining] = useState(false);
  const [step, setStep] = useState<"input" | "preview">("input");

  useEffect(() => {
    if (open) {
      fetchWorkspaces();
      resetState();
    }
  }, [open]);

  const resetState = () => {
    setSelectedWorkspace("");
    setPrompt("");
    setRefinedTitle("");
    setRefinedDescription("");
    setStep("input");
    setLoading(false);
    setRefining(false);
  };

  const fetchWorkspaces = async () => {
    try {
      const res = await fetch("/api/workspaces");
      const data = await res.json();
      setWorkspaces(data);
    } catch (error) {
      console.error("Failed to fetch workspaces:", error);
    }
  };

  const handleRefine = async () => {
    if (!selectedWorkspace || !prompt.trim()) return;

    setRefining(true);
    try {
      const res = await fetch("/api/manager/summary", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          workspaceId: parseInt(selectedWorkspace),
          prompt: prompt.trim(),
        }),
      });

      if (res.ok) {
        const data = await res.json();
        setRefinedTitle(data.title || prompt.slice(0, 50));
        setRefinedDescription(data.description || prompt);
        setStep("preview");
      } else {
        // Fallback to using prompt as-is
        setRefinedTitle(prompt.slice(0, 100));
        setRefinedDescription(prompt);
        setStep("preview");
      }
    } catch (error) {
      console.error("Failed to refine task:", error);
      // Fallback to using prompt as-is
      setRefinedTitle(prompt.slice(0, 100));
      setRefinedDescription(prompt);
      setStep("preview");
    } finally {
      setRefining(false);
    }
  };

  const handleCreate = async () => {
    if (!selectedWorkspace) return;

    setLoading(true);
    try {
      await fetch("/api/tasks", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          workspaceId: parseInt(selectedWorkspace),
          title: refinedTitle,
          description: refinedDescription,
          state: "new",
        }),
      });
      onTaskCreated();
      onClose();
    } catch (error) {
      console.error("Failed to create task:", error);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {step === "input" ? "Create New Task" : "Review Task"}
          </DialogTitle>
        </DialogHeader>

        {step === "input" ? (
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label>Workspace</Label>
              <Select
                value={selectedWorkspace}
                onValueChange={setSelectedWorkspace}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select a workspace..." />
                </SelectTrigger>
                <SelectContent>
                  {workspaces.map((ws) => (
                    <SelectItem key={ws.id} value={ws.id.toString()}>
                      {ws.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>What do you want to build?</Label>
              <Textarea
                placeholder="Describe the feature, bug fix, or task..."
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                rows={4}
              />
              <p className="text-xs text-muted-foreground">
                A manager bot will analyze your codebase and refine this into a task.
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label>Title</Label>
              <Textarea
                value={refinedTitle}
                onChange={(e) => setRefinedTitle(e.target.value)}
                rows={2}
              />
            </div>

            <div className="space-y-2">
              <Label>Description</Label>
              <Textarea
                value={refinedDescription}
                onChange={(e) => setRefinedDescription(e.target.value)}
                rows={6}
              />
            </div>

            <p className="text-xs text-muted-foreground">
              Review and edit the task details before creating.
            </p>
          </div>
        )}

        <DialogFooter>
          {step === "input" ? (
            <>
              <Button variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button
                onClick={handleRefine}
                disabled={!selectedWorkspace || !prompt.trim() || refining}
              >
                {refining ? (
                  <>
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                    Refining...
                  </>
                ) : (
                  "Continue"
                )}
              </Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={() => setStep("input")}>
                Back
              </Button>
              <Button onClick={handleCreate} disabled={loading}>
                {loading ? (
                  <>
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                    Creating...
                  </>
                ) : (
                  "Create Task"
                )}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
