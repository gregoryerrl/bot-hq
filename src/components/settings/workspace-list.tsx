"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Trash2, FolderGit2, Settings } from "lucide-react";
import { Workspace } from "@/lib/db/schema";
import Link from "next/link";

interface WorkspaceListProps {
  onAddClick: () => void;
}

export function WorkspaceList({ onAddClick }: WorkspaceListProps) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchWorkspaces();
  }, []);

  async function fetchWorkspaces() {
    try {
      const res = await fetch("/api/workspaces");
      const data = await res.json();
      setWorkspaces(data);
    } catch (error) {
      console.error("Failed to fetch workspaces:", error);
    } finally {
      setLoading(false);
    }
  }

  async function deleteWorkspace(id: number) {
    if (!confirm("Delete this workspace?")) return;

    try {
      await fetch(`/api/workspaces/${id}`, { method: "DELETE" });
      setWorkspaces(workspaces.filter((w) => w.id !== id));
    } catch (error) {
      console.error("Failed to delete workspace:", error);
    }
  }

  if (loading) {
    return <div className="text-muted-foreground">Loading workspaces...</div>;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold">Workspaces</h3>
        <Button onClick={onAddClick} size="sm">
          Add Workspace
        </Button>
      </div>

      {workspaces.length === 0 ? (
        <Card className="p-6 text-center text-muted-foreground">
          No workspaces configured
        </Card>
      ) : (
        <div className="space-y-2">
          {workspaces.map((workspace) => (
            <Card key={workspace.id} className="p-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <FolderGit2 className="h-5 w-5 text-muted-foreground" />
                  <div>
                    <div className="font-medium">{workspace.name}</div>
                    <div className="text-sm text-muted-foreground">
                      {workspace.repoPath}
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <Button variant="ghost" size="icon" asChild>
                    <Link href={`/settings/workspaces/${workspace.id}`}>
                      <Settings className="h-4 w-4" />
                    </Link>
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => deleteWorkspace(workspace.id)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
