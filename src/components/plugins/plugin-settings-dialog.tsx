"use client";

import { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface PluginSettingsDialogProps {
  pluginName: string | null;
  open: boolean;
  onClose: () => void;
}

interface PluginDetails {
  name: string;
  version: string;
  description: string;
  manifest: {
    settings?: Record<string, {
      type: string;
      label: string;
      description?: string;
      default?: unknown;
    }>;
    credentials?: Record<string, {
      type: string;
      label: string;
      description?: string;
      required?: boolean;
    }>;
  };
  settings: Record<string, unknown>;
}

export function PluginSettingsDialog({
  pluginName,
  open,
  onClose,
}: PluginSettingsDialogProps) {
  const [plugin, setPlugin] = useState<PluginDetails | null>(null);
  const [settings, setSettings] = useState<Record<string, unknown>>({});
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open && pluginName) {
      fetchPluginDetails();
    }
  }, [open, pluginName]);

  const fetchPluginDetails = async () => {
    if (!pluginName) return;
    setLoading(true);
    try {
      const res = await fetch(`/api/plugins/${pluginName}`);
      const data = await res.json();
      setPlugin(data);
      setSettings(data.settings || {});
      // Don't pre-fill credentials for security
      setCredentials({});
    } catch (error) {
      console.error("Failed to fetch plugin details:", error);
    } finally {
      setLoading(false);
    }
  };

  const handleSave = async () => {
    if (!pluginName) return;
    setSaving(true);
    try {
      await fetch(`/api/plugins/${pluginName}/settings`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ settings, credentials }),
      });
      onClose();
    } catch (error) {
      console.error("Failed to save settings:", error);
    } finally {
      setSaving(false);
    }
  };

  const hasSettings = plugin?.manifest.settings && Object.keys(plugin.manifest.settings).length > 0;
  const hasCredentials = plugin?.manifest.credentials && Object.keys(plugin.manifest.credentials).length > 0;

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {plugin?.name} Settings
          </DialogTitle>
        </DialogHeader>

        {loading ? (
          <div className="py-8 text-center text-muted-foreground">
            Loading...
          </div>
        ) : plugin ? (
          <div className="space-y-6 py-4">
            {/* Settings Section */}
            {hasSettings && (
              <div className="space-y-4">
                <h4 className="text-sm font-medium">Settings</h4>
                {Object.entries(plugin.manifest.settings!).map(([key, def]) => (
                  <div key={key} className="space-y-2">
                    <Label htmlFor={`setting-${key}`}>{def.label}</Label>
                    <Input
                      id={`setting-${key}`}
                      value={String(settings[key] ?? def.default ?? "")}
                      onChange={(e) =>
                        setSettings({ ...settings, [key]: e.target.value })
                      }
                      placeholder={def.description}
                    />
                    {def.description && (
                      <p className="text-xs text-muted-foreground">
                        {def.description}
                      </p>
                    )}
                  </div>
                ))}
              </div>
            )}

            {/* Credentials Section */}
            {hasCredentials && (
              <div className="space-y-4">
                <h4 className="text-sm font-medium">Credentials</h4>
                {Object.entries(plugin.manifest.credentials!).map(([key, def]) => (
                  <div key={key} className="space-y-2">
                    <Label htmlFor={`cred-${key}`}>
                      {def.label}
                      {def.required && <span className="text-red-500 ml-1">*</span>}
                    </Label>
                    <Input
                      id={`cred-${key}`}
                      type="password"
                      value={credentials[key] ?? ""}
                      onChange={(e) =>
                        setCredentials({ ...credentials, [key]: e.target.value })
                      }
                      placeholder={def.description || "Enter value..."}
                    />
                    {def.description && (
                      <p className="text-xs text-muted-foreground">
                        {def.description}
                      </p>
                    )}
                  </div>
                ))}
              </div>
            )}

            {!hasSettings && !hasCredentials && (
              <p className="text-sm text-muted-foreground text-center py-4">
                This plugin has no configurable settings.
              </p>
            )}
          </div>
        ) : null}

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSave} disabled={saving || loading}>
            {saving ? "Saving..." : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
