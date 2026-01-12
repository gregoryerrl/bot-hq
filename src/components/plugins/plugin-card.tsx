"use client";

import { useState } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Settings, Puzzle } from "lucide-react";

interface Plugin {
  name: string;
  version: string;
  description: string;
  enabled: boolean;
  hasUI: boolean;
  hasMcp: boolean;
}

interface PluginCardProps {
  plugin: Plugin;
  onToggleEnabled: (name: string, enabled: boolean) => void;
  onOpenSettings: (name: string) => void;
}

export function PluginCard({
  plugin,
  onToggleEnabled,
  onOpenSettings,
}: PluginCardProps) {
  const [loading, setLoading] = useState(false);

  const handleToggle = async (checked: boolean) => {
    setLoading(true);
    await onToggleEnabled(plugin.name, checked);
    setLoading(false);
  };

  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-start gap-3">
          <div className="p-2 rounded-lg bg-muted">
            <Puzzle className="h-5 w-5" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <h3 className="font-medium">{plugin.name}</h3>
              <Badge variant="outline" className="text-xs">
                v{plugin.version}
              </Badge>
              {plugin.hasMcp && (
                <Badge variant="secondary" className="text-xs">
                  MCP
                </Badge>
              )}
              {plugin.hasUI && (
                <Badge variant="secondary" className="text-xs">
                  UI
                </Badge>
              )}
            </div>
            <p className="text-sm text-muted-foreground mt-1">
              {plugin.description}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="icon"
            variant="ghost"
            onClick={() => onOpenSettings(plugin.name)}
            disabled={!plugin.enabled}
          >
            <Settings className="h-4 w-4" />
          </Button>
          <Switch
            checked={plugin.enabled}
            onCheckedChange={handleToggle}
            disabled={loading}
          />
        </div>
      </div>
    </Card>
  );
}
