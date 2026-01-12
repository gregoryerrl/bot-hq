"use client";

import { useState, useEffect } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Puzzle } from "lucide-react";

interface PluginAction {
  pluginName: string;
  id: string;
  label: string;
  description?: string;
  icon?: string;
  defaultChecked: boolean;
}

interface PluginActionCheckboxesProps {
  type: "approval" | "task";
  selectedActions: string[];
  onSelectionChange: (selected: string[]) => void;
}

export function PluginActionCheckboxes({
  type,
  selectedActions,
  onSelectionChange,
}: PluginActionCheckboxesProps) {
  const [actions, setActions] = useState<PluginAction[]>([]);
  const [loading, setLoading] = useState(true);
  const [initialized, setInitialized] = useState(false);

  useEffect(() => {
    fetchActions();
  }, [type]);

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

  const fetchActions = async () => {
    try {
      const res = await fetch(`/api/plugins/actions?type=${type}`);
      const data = await res.json();
      setActions(data.actions || []);
    } catch (error) {
      console.error("Failed to fetch plugin actions:", error);
    } finally {
      setLoading(false);
    }
  };

  const handleToggle = (pluginName: string, actionId: string, checked: boolean) => {
    const key = `${pluginName}:${actionId}`;
    if (checked) {
      onSelectionChange([...selectedActions, key]);
    } else {
      onSelectionChange(selectedActions.filter(k => k !== key));
    }
  };

  if (loading) {
    return null; // Don't show loading state, just nothing
  }

  if (actions.length === 0) {
    return null; // No plugin actions available
  }

  return (
    <div className="space-y-3">
      <p className="text-sm font-medium">On Accept, also:</p>
      {actions.map((action) => {
        const key = `${action.pluginName}:${action.id}`;
        const isChecked = selectedActions.includes(key);

        return (
          <div
            key={key}
            className="flex items-start gap-3 p-3 rounded-lg border bg-muted/30"
          >
            <Checkbox
              id={key}
              checked={isChecked}
              onCheckedChange={(checked) =>
                handleToggle(action.pluginName, action.id, checked === true)
              }
            />
            <div className="flex-1 min-w-0">
              <Label
                htmlFor={key}
                className="flex items-center gap-2 cursor-pointer"
              >
                <Puzzle className="h-4 w-4 text-muted-foreground" />
                <span className="text-xs text-muted-foreground">
                  {action.pluginName}
                </span>
                <span>{action.label}</span>
              </Label>
              {action.description && (
                <p className="text-xs text-muted-foreground mt-1 ml-6">
                  {action.description}
                </p>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
