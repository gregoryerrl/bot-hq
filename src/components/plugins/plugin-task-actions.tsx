"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MoreVertical, Loader2 } from "lucide-react";
import { toast } from "sonner";

interface PluginTaskAction {
  pluginName: string;
  id: string;
  label: string;
  description?: string;
  icon?: string;
}

interface PluginTaskActionsProps {
  taskId: number;
  workspaceId: number;
}

export function PluginTaskActions({ taskId, workspaceId }: PluginTaskActionsProps) {
  const [actions, setActions] = useState<PluginTaskAction[]>([]);
  const [loading, setLoading] = useState(true);
  const [executing, setExecuting] = useState<string | null>(null);

  useEffect(() => {
    async function fetchActions() {
      try {
        const res = await fetch("/api/plugins/actions?type=task");
        if (!res.ok) throw new Error("Failed to fetch");
        const data = await res.json();
        setActions(data.actions || []);
      } catch (error) {
        console.error("Failed to load plugin task actions:", error);
      } finally {
        setLoading(false);
      }
    }

    fetchActions();
  }, []);

  async function executeAction(action: PluginTaskAction) {
    setExecuting(action.id);
    try {
      const res = await fetch("/api/plugins/actions/execute", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          pluginName: action.pluginName,
          actionId: action.id,
          taskId,
          workspaceId,
        }),
      });

      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Action failed");
      }

      const result = await res.json();
      toast.success(result.message || `${action.label} completed`);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Action failed");
    } finally {
      setExecuting(null);
    }
  }

  if (loading || actions.length === 0) {
    return null;
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm" className="h-8 w-8 p-0">
          <MoreVertical className="h-4 w-4" />
          <span className="sr-only">Plugin actions</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {actions.map((action) => (
          <DropdownMenuItem
            key={`${action.pluginName}-${action.id}`}
            onClick={() => executeAction(action)}
            disabled={executing !== null}
          >
            {executing === action.id ? (
              <Loader2 className="h-4 w-4 mr-2 animate-spin" />
            ) : null}
            <span className="text-xs text-muted-foreground mr-2">
              [{action.pluginName}]
            </span>
            {action.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
