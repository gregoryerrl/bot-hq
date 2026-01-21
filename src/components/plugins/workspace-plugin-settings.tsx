"use client";

import { useState, useEffect } from "react";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import { usePluginUI } from "@/hooks/use-plugin-ui";
import { GitHubWorkspaceSettings } from "./github-workspace-settings";

interface WorkspacePluginSettingsProps {
  workspaceId: number;
}

export function WorkspacePluginSettings({ workspaceId }: WorkspacePluginSettingsProps) {
  const { workspaceSettings, loading: loadingUI } = usePluginUI();
  const [pluginData, setPluginData] = useState<Record<string, Record<string, unknown>>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    async function fetchData() {
      if (workspaceSettings.length === 0) {
        setLoading(false);
        return;
      }

      try {
        const results: Record<string, Record<string, unknown>> = {};
        for (const setting of workspaceSettings) {
          const res = await fetch(
            `/api/plugins/${setting.pluginName}/workspace-data/${workspaceId}`
          );
          if (res.ok) {
            results[setting.pluginName] = await res.json();
          } else {
            results[setting.pluginName] = {};
          }
        }
        setPluginData(results);
      } catch (error) {
        console.error("Failed to fetch plugin data:", error);
      } finally {
        setLoading(false);
      }
    }

    if (!loadingUI) {
      fetchData();
    }
  }, [workspaceId, workspaceSettings, loadingUI]);

  async function savePluginData(pluginName: string) {
    setSaving(pluginName);
    try {
      const res = await fetch(
        `/api/plugins/${pluginName}/workspace-data/${workspaceId}`,
        {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(pluginData[pluginName] || {}),
        }
      );

      if (!res.ok) throw new Error("Failed to save");
      toast.success(`${pluginName} settings saved`);
    } catch {
      toast.error("Failed to save settings");
    } finally {
      setSaving(null);
    }
  }

  if (loadingUI || loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (workspaceSettings.length === 0) {
    return null;
  }

  return (
    <div className="space-y-6">
      {workspaceSettings.map((setting) => {
        // Use specialized components for known plugins
        if (setting.pluginName === "github") {
          return <GitHubWorkspaceSettings key={setting.pluginName} workspaceId={workspaceId} />;
        }

        // Generic fallback for other plugins
        return (
          <Card key={setting.pluginName}>
            <CardHeader>
              <CardTitle className="capitalize">{setting.pluginName} Settings</CardTitle>
              <CardDescription>
                Configure {setting.pluginName} for this workspace
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <p className="text-sm text-muted-foreground">
                Plugin-specific settings will appear here when the plugin provides a settings component.
              </p>
              <div className="flex justify-end">
                <Button
                  size="sm"
                  onClick={() => savePluginData(setting.pluginName)}
                  disabled={saving !== null}
                >
                  {saving === setting.pluginName ? (
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                  ) : null}
                  Save
                </Button>
              </div>
            </CardContent>
          </Card>
        );
      })}
    </div>
  );
}
