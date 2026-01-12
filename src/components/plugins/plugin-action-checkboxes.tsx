"use client";

import { useState, useEffect, useCallback } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Puzzle, Loader2, CheckCircle, XCircle, AlertCircle } from "lucide-react";
import { cn } from "@/lib/utils";

interface PluginAction {
  pluginName: string;
  id: string;
  label: string;
  description?: string;
  icon?: string;
  defaultChecked: boolean;
}

export type ActionStatus = "idle" | "pending" | "executing" | "success" | "error";

export interface ActionStatusMap {
  [key: string]: {
    status: ActionStatus;
    message?: string;
    error?: string;
  };
}

interface PluginActionCheckboxesProps {
  type: "approval" | "task";
  selectedActions: string[];
  onSelectionChange: (selected: string[]) => void;
  actionStatuses?: ActionStatusMap;
  disabled?: boolean;
}

export function PluginActionCheckboxes({
  type,
  selectedActions,
  onSelectionChange,
  actionStatuses = {},
  disabled = false,
}: PluginActionCheckboxesProps) {
  const [actions, setActions] = useState<PluginAction[]>([]);
  const [loading, setLoading] = useState(true);
  const [initialized, setInitialized] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);

  const fetchActions = useCallback(async () => {
    try {
      setFetchError(null);
      const res = await fetch(`/api/plugins/actions?type=${type}`);
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      const data = await res.json();
      setActions(data.actions || []);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Unknown error";
      console.error("Failed to fetch plugin actions:", message);
      setFetchError(`Failed to load plugin actions: ${message}`);
    } finally {
      setLoading(false);
    }
  }, [type]);

  useEffect(() => {
    fetchActions();
  }, [fetchActions]);

  useEffect(() => {
    // Initialize with default checked actions only once
    if (actions.length > 0 && !initialized) {
      const defaults = actions
        .filter(a => a.defaultChecked)
        .map(a => `${a.pluginName}:${a.id}`);
      if (defaults.length > 0) {
        onSelectionChange(defaults);
      }
      setInitialized(true);
    }
  }, [actions, initialized, onSelectionChange]);

  const handleToggle = (pluginName: string, actionId: string, checked: boolean) => {
    if (disabled) return;
    const key = `${pluginName}:${actionId}`;
    if (checked) {
      onSelectionChange([...selectedActions, key]);
    } else {
      onSelectionChange(selectedActions.filter(k => k !== key));
    }
  };

  const getStatusIcon = (key: string) => {
    const statusInfo = actionStatuses[key];
    if (!statusInfo) return null;

    switch (statusInfo.status) {
      case "pending":
        return <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />;
      case "executing":
        return <Loader2 className="h-4 w-4 animate-spin text-blue-500" />;
      case "success":
        return <CheckCircle className="h-4 w-4 text-green-500" />;
      case "error":
        return <XCircle className="h-4 w-4 text-destructive" />;
      default:
        return null;
    }
  };

  const getStatusClass = (key: string) => {
    const statusInfo = actionStatuses[key];
    if (!statusInfo) return "";

    switch (statusInfo.status) {
      case "executing":
        return "border-blue-500/50 bg-blue-500/5";
      case "success":
        return "border-green-500/50 bg-green-500/5";
      case "error":
        return "border-destructive/50 bg-destructive/5";
      default:
        return "";
    }
  };

  if (loading) {
    return null;
  }

  if (fetchError) {
    return (
      <div className="flex items-center gap-2 p-3 rounded-lg border border-amber-500/50 bg-amber-500/5 text-sm">
        <AlertCircle className="h-4 w-4 text-amber-500 shrink-0" />
        <span className="text-muted-foreground">{fetchError}</span>
      </div>
    );
  }

  if (actions.length === 0) {
    return null;
  }

  return (
    <div className="space-y-3">
      <p className="text-sm font-medium">On Accept, also:</p>
      {actions.map((action) => {
        const key = `${action.pluginName}:${action.id}`;
        const isChecked = selectedActions.includes(key);
        const statusInfo = actionStatuses[key];
        const isExecuting = statusInfo?.status === "executing" || statusInfo?.status === "pending";

        return (
          <div
            key={key}
            className={cn(
              "flex items-start gap-3 p-3 rounded-lg border bg-muted/30 transition-colors",
              getStatusClass(key),
              disabled && "opacity-60"
            )}
          >
            <Checkbox
              id={key}
              checked={isChecked}
              onCheckedChange={(checked) =>
                handleToggle(action.pluginName, action.id, checked === true)
              }
              disabled={disabled || isExecuting}
            />
            <div className="flex-1 min-w-0">
              <Label
                htmlFor={key}
                className={cn(
                  "flex items-center gap-2",
                  !disabled && !isExecuting && "cursor-pointer"
                )}
              >
                <Puzzle className="h-4 w-4 text-muted-foreground" />
                <span className="text-xs text-muted-foreground">
                  {action.pluginName}
                </span>
                <span>{action.label}</span>
                {getStatusIcon(key)}
              </Label>
              {action.description && (
                <p className="text-xs text-muted-foreground mt-1 ml-6">
                  {action.description}
                </p>
              )}
              {statusInfo?.status === "success" && statusInfo.message && (
                <p className="text-xs text-green-600 mt-1 ml-6">
                  {statusInfo.message}
                </p>
              )}
              {statusInfo?.status === "error" && statusInfo.error && (
                <p className="text-xs text-destructive mt-1 ml-6">
                  {statusInfo.error}
                </p>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
