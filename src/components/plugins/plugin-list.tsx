"use client";

import { useState, useEffect, useCallback } from "react";
import { PluginCard } from "./plugin-card";
import { PluginSettingsDialog } from "./plugin-settings-dialog";
import { Puzzle } from "lucide-react";

interface Plugin {
  name: string;
  version: string;
  description: string;
  enabled: boolean;
  hasUI: boolean;
  hasMcp: boolean;
}

export function PluginList() {
  const [plugins, setPlugins] = useState<Plugin[]>([]);
  const [loading, setLoading] = useState(true);
  const [settingsPlugin, setSettingsPlugin] = useState<string | null>(null);

  const fetchPlugins = useCallback(async () => {
    try {
      const res = await fetch("/api/plugins");
      const data = await res.json();
      setPlugins(data.plugins || []);
    } catch (error) {
      console.error("Failed to fetch plugins:", error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchPlugins();
  }, [fetchPlugins]);

  const handleToggleEnabled = async (name: string, enabled: boolean) => {
    try {
      await fetch(`/api/plugins/${name}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ enabled }),
      });
      // Update local state optimistically
      setPlugins(plugins.map(p =>
        p.name === name ? { ...p, enabled } : p
      ));
    } catch (error) {
      console.error("Failed to toggle plugin:", error);
      // Refetch to get correct state
      fetchPlugins();
    }
  };

  if (loading) {
    return (
      <div className="text-muted-foreground">Loading plugins...</div>
    );
  }

  if (plugins.length === 0) {
    return (
      <div className="rounded-lg border border-dashed p-8 text-center">
        <Puzzle className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
        <h3 className="font-medium mb-2">No plugins installed</h3>
        <p className="text-sm text-muted-foreground">
          Install plugins by adding them to ~/.bot-hq/plugins/
        </p>
      </div>
    );
  }

  return (
    <>
      <div className="space-y-3">
        {plugins.map((plugin) => (
          <PluginCard
            key={plugin.name}
            plugin={plugin}
            onToggleEnabled={handleToggleEnabled}
            onOpenSettings={setSettingsPlugin}
          />
        ))}
      </div>

      <PluginSettingsDialog
        pluginName={settingsPlugin}
        open={!!settingsPlugin}
        onClose={() => setSettingsPlugin(null)}
      />
    </>
  );
}
