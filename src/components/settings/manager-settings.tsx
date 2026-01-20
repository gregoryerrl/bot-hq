"use client";

import { useState, useEffect } from "react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Save, RefreshCw, Loader2 } from "lucide-react";
import { toast } from "sonner";

interface ManagerSettings {
  managerPrompt: string;
  maxIterations: number;
  stuckThreshold: number;
}

export function ManagerSettings() {
  const [settings, setSettings] = useState<ManagerSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetchSettings();
  }, []);

  const fetchSettings = async () => {
    try {
      setLoading(true);
      const response = await fetch("/api/manager-settings");
      if (!response.ok) throw new Error("Failed to fetch settings");
      const data = await response.json();
      setSettings(data);
    } catch (error) {
      toast.error("Failed to load manager settings");
    } finally {
      setLoading(false);
    }
  };

  const saveSettings = async () => {
    if (!settings) return;

    try {
      setSaving(true);
      const response = await fetch("/api/manager-settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(settings),
      });

      if (!response.ok) throw new Error("Failed to save settings");

      toast.success("Manager settings updated successfully");
    } catch (error) {
      toast.error("Failed to save settings");
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        <span className="ml-2 text-muted-foreground">Loading settings...</span>
      </div>
    );
  }

  if (!settings) return null;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-semibold">Manager Configuration</h3>
          <p className="text-sm text-muted-foreground">
            Configure the persistent Claude Code manager session
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={fetchSettings}>
            <RefreshCw className="h-4 w-4 mr-2" />
            Refresh
          </Button>
          <Button size="sm" onClick={saveSettings} disabled={saving}>
            <Save className="h-4 w-4 mr-2" />
            {saving ? "Saving..." : "Save"}
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Iteration Settings</CardTitle>
          <CardDescription>Control how subagents iterate on tasks</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="maxIterations">Max Iterations</Label>
              <Input
                id="maxIterations"
                type="number"
                min={1}
                max={50}
                value={settings.maxIterations}
                onChange={(e) =>
                  setSettings({ ...settings, maxIterations: parseInt(e.target.value) || 10 })
                }
              />
              <p className="text-xs text-muted-foreground">
                Maximum attempts before escalating to needs_help
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="stuckThreshold">Stuck Threshold</Label>
              <Input
                id="stuckThreshold"
                type="number"
                min={1}
                max={10}
                value={settings.stuckThreshold}
                onChange={(e) =>
                  setSettings({ ...settings, stuckThreshold: parseInt(e.target.value) || 3 })
                }
              />
              <p className="text-xs text-muted-foreground">
                Same blocker N times triggers early escalation
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Manager Prompt</CardTitle>
          <CardDescription>
            Instructions given to the manager on startup (MANAGER_PROMPT.md)
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Textarea
            value={settings.managerPrompt}
            onChange={(e) => setSettings({ ...settings, managerPrompt: e.target.value })}
            className="min-h-[400px] font-mono text-sm"
            placeholder="Enter manager prompt..."
          />
        </CardContent>
      </Card>
    </div>
  );
}
