"use client";

import { Badge } from "@/components/ui/badge";
import { usePluginUI } from "@/hooks/use-plugin-ui";

interface PluginTaskBadgesProps {
  taskId: number;
  workspaceId: number;
}

export function PluginTaskBadges({ taskId, workspaceId }: PluginTaskBadgesProps) {
  const { taskBadges, loading } = usePluginUI();

  if (loading || taskBadges.length === 0) {
    return null;
  }

  // For now, render placeholder badges
  // When plugins provide actual components, we'll render those
  return (
    <>
      {taskBadges.map((badge) => (
        <Badge
          key={badge.pluginName}
          variant="outline"
          className="text-xs"
          title={`From ${badge.pluginName} plugin`}
        >
          {badge.pluginName}
        </Badge>
      ))}
    </>
  );
}
