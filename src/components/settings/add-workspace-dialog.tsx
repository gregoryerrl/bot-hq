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
  const [githubRemote, setGithubRemote] = useState("");
  const [buildCommand, setBuildCommand] = useState("");
  const [loading, setLoading] = useState(false);

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
          githubRemote: githubRemote || null,
          buildCommand: buildCommand || null,
        }),
      });

      if (res.ok) {
        setName("");
        setRepoPath("");
        setGithubRemote("");
        setBuildCommand("");
        onSuccess();
        onOpenChange(false);
      }
    } catch (error) {
      console.error("Failed to create workspace:", error);
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
            <Input
              value={repoPath}
              onChange={(e) => setRepoPath(e.target.value)}
              placeholder="~/Projects/my-project"
              required
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">
              GitHub Remote (optional)
            </label>
            <Input
              value={githubRemote}
              onChange={(e) => setGithubRemote(e.target.value)}
              placeholder="owner/repo"
            />
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
