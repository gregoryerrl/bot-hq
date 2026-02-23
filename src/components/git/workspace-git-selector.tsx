"use client";

import { useState, useEffect } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FolderGit2 } from "lucide-react";

interface Workspace {
  id: number;
  name: string;
  repoPath: string;
}

interface WorkspaceGitSelectorProps {
  value: string;
  onChange: (workspaceId: string) => void;
}

export function WorkspaceGitSelector({
  value,
  onChange,
}: WorkspaceGitSelectorProps) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);

  useEffect(() => {
    fetch("/api/workspaces")
      .then((res) => res.json())
      .then((data) => {
        const list = Array.isArray(data) ? data : data.workspaces || [];
        setWorkspaces(list);
        // Auto-select first if none selected
        if (!value && list.length > 0) {
          onChange(String(list[0].id));
        }
      })
      .catch(() => {});
  }, []);

  if (workspaces.length === 0) {
    return (
      <div className="text-sm text-muted-foreground flex items-center gap-2">
        <FolderGit2 className="h-4 w-4" />
        No workspaces configured
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <FolderGit2 className="h-4 w-4 text-muted-foreground" />
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger className="w-[220px]">
          <SelectValue placeholder="Select workspace..." />
        </SelectTrigger>
        <SelectContent>
          {workspaces.map((ws) => (
            <SelectItem key={ws.id} value={String(ws.id)}>
              {ws.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
