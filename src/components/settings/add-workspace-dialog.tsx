"use client";

import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { FolderOpen } from "lucide-react";
import { toast } from "sonner";

interface AddWorkspaceDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSuccess: () => void;
}

export function AddWorkspaceDialog({
  open,
  onOpenChange,
  onSuccess,
}: AddWorkspaceDialogProps) {
  const [name, setName] = useState("");
  const [repoPath, setRepoPath] = useState("");
  const [buildCommand, setBuildCommand] = useState("");
  const [loading, setLoading] = useState(false);

  async function handleFolderPicker() {
    try {
      // Get scope path from API
      const scopeRes = await fetch("/api/settings?key=scope_path");
      const scopeData = await scopeRes.json();
      const scopePath = scopeData.value;

      // Call the folder picker API
      const res = await fetch("/api/pick-folder", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ startPath: scopePath }),
      });

      if (res.ok) {
        const data = await res.json();
        if (data.path) {
          setRepoPath(data.path);
        }
      }
    } catch (error) {
      console.error("Failed to pick folder:", error);
      toast.error("Failed to open folder picker");
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setLoading(true);

    try {
      const res = await fetch("/api/workspaces", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name,
          repoPath,
          buildCommand: buildCommand || null,
        }),
      });

      if (res.ok) {
        setName("");
        setRepoPath("");
        setBuildCommand("");
        onSuccess();
        onOpenChange(false);
        toast.success("Workspace created successfully");
      } else {
        const error = await res.json();
        toast.error(error.error || "Failed to create workspace");
      }
    } catch (error) {
      console.error("Failed to create workspace:", error);
      toast.error("Failed to create workspace");
    } finally {
      setLoading(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Workspace</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">Name</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-project"
              required
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">Repository Path</label>
            <div className="flex gap-2">
              <Input
                value={repoPath}
                onChange={(e) => setRepoPath(e.target.value)}
                placeholder="~/Projects/my-project"
                required
                className="flex-1"
              />
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={handleFolderPicker}
                title="Browse folders"
              >
                <FolderOpen className="h-4 w-4" />
              </Button>
            </div>
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">
              Build Command (optional)
            </label>
            <Input
              value={buildCommand}
              onChange={(e) => setBuildCommand(e.target.value)}
              placeholder="npm run build"
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={loading}>
              {loading ? "Adding..." : "Add Workspace"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
