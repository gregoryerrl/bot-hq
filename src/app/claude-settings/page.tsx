"use client";

import { useState, useEffect } from "react";
import { Header } from "@/components/layout/header";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Package, Puzzle, Shield, User, RefreshCw, Save, AlertCircle, Settings, Store } from "lucide-react";

interface ClaudeSettings {
  configPath: string;
  mcpServers: MCPServer[];
  skills: Skill[];
  plugins: Plugin[];
  marketplaces: Marketplace[];
  accountInfo: AccountInfo;
  permissions: PermissionInfo;
  globalPreferences: GlobalPreferences;
}

interface Marketplace {
  name: string;
  repo: string;
  installLocation: string;
  lastUpdated: string;
}

interface MCPServer {
  name: string;
  command: string;
  args: string[];
  env?: Record<string, string>;
  enabled: boolean;
}

interface Skill {
  name: string;
  description: string;
  location: string;
  type: "user" | "plugin";
}

interface Plugin {
  name: string;
  version: string;
  description: string;
  enabled: boolean;
}

interface AccountInfo {
  apiKeyConfigured: boolean;
  model: string;
  organization?: string;
}

interface PermissionInfo {
  autoApproveEdits: boolean;
  allowedCommands: string[];
  blockedCommands: string[];
}

interface GlobalPreferences {
  theme: string;
  verbose: boolean;
  outputFormat: string;
}

export default function ClaudeSettingsPage() {
  const [settings, setSettings] = useState<ClaudeSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetchSettings();
  }, []);

  const fetchSettings = async () => {
    try {
      setLoading(true);
      setError(null);
      const response = await fetch("/api/claude-settings");
      if (!response.ok) {
        throw new Error("Failed to fetch Claude Code settings");
      }
      const data = await response.json();
      setSettings(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
    } finally {
      setLoading(false);
    }
  };

  const handleSave = async () => {
    if (!settings) return;

    try {
      setSaving(true);
      const response = await fetch("/api/claude-settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(settings),
      });

      if (!response.ok) {
        throw new Error("Failed to save settings");
      }

      await fetchSettings();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="flex-1 overflow-y-auto">
        <Header title="Claude Code Settings" />
        <main className="p-6">
          <div className="flex items-center justify-center py-12">
            <RefreshCw className="h-6 w-6 animate-spin text-muted-foreground" />
            <span className="ml-2 text-muted-foreground">Loading settings...</span>
          </div>
        </main>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex-1 overflow-y-auto">
        <Header title="Claude Code Settings" />
        <main className="p-6">
          <Card className="border-destructive">
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-destructive">
                <AlertCircle className="h-5 w-5" />
                Error Loading Settings
              </CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-sm text-muted-foreground mb-4">{error}</p>
              <Button onClick={fetchSettings}>Retry</Button>
            </CardContent>
          </Card>
        </main>
      </div>
    );
  }

  if (!settings) return null;

  return (
    <div className="flex-1 overflow-y-auto">
      <Header title="Claude Code Settings" />

      <main className="p-6">
        <div className="max-w-6xl mx-auto space-y-6">
          <div className="flex items-center justify-between">
            <div>
              <h2 className="text-2xl font-bold">Global Claude Code Configuration</h2>
              <p className="text-sm text-muted-foreground mt-1">
                Manage your machine-wide Claude Code settings, MCP servers, skills, and more
              </p>
            </div>
            <div className="flex gap-2">
              <Button variant="outline" onClick={fetchSettings}>
                <RefreshCw className="h-4 w-4 mr-2" />
                Refresh
              </Button>
              <Button onClick={handleSave} disabled={saving}>
                <Save className="h-4 w-4 mr-2" />
                {saving ? "Saving..." : "Save All"}
              </Button>
            </div>
          </div>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm font-medium text-muted-foreground">
                Config Location
              </CardTitle>
            </CardHeader>
            <CardContent>
              <code className="text-xs bg-muted px-2 py-1 rounded">
                {settings.configPath || "~/.claude/settings.json"}
              </code>
            </CardContent>
          </Card>

          <Tabs defaultValue="mcp" className="space-y-4">
            <TabsList className="grid w-full grid-cols-7 md:grid-cols-7">
              <TabsTrigger value="mcp">
                <Package className="h-4 w-4 mr-2" />
                <span className="hidden md:inline">MCP Servers</span>
                <span className="md:hidden">MCP</span>
              </TabsTrigger>
              <TabsTrigger value="skills">
                <Puzzle className="h-4 w-4 mr-2" />
                Skills
              </TabsTrigger>
              <TabsTrigger value="plugins">
                <Settings className="h-4 w-4 mr-2" />
                Plugins
              </TabsTrigger>
              <TabsTrigger value="marketplaces">
                <Store className="h-4 w-4 mr-2" />
                <span className="hidden md:inline">Marketplaces</span>
                <span className="md:hidden">Markets</span>
              </TabsTrigger>
              <TabsTrigger value="permissions">
                <Shield className="h-4 w-4 mr-2" />
                <span className="hidden md:inline">Permissions</span>
                <span className="md:hidden">Perms</span>
              </TabsTrigger>
              <TabsTrigger value="account">
                <User className="h-4 w-4 mr-2" />
                Account
              </TabsTrigger>
              <TabsTrigger value="preferences">
                <Settings className="h-4 w-4 mr-2" />
                <span className="hidden md:inline">Preferences</span>
                <span className="md:hidden">Prefs</span>
              </TabsTrigger>
            </TabsList>

            <TabsContent value="mcp" className="space-y-4">
              <MCPServersTab
                servers={settings.mcpServers}
                onChange={(servers) => setSettings({ ...settings, mcpServers: servers })}
              />
            </TabsContent>

            <TabsContent value="skills" className="space-y-4">
              <SkillsTab skills={settings.skills} />
            </TabsContent>

            <TabsContent value="plugins" className="space-y-4">
              <PluginsTab
                plugins={settings.plugins}
                onChange={(plugins) => setSettings({ ...settings, plugins })}
              />
            </TabsContent>

            <TabsContent value="marketplaces" className="space-y-4">
              <MarketplacesTab marketplaces={settings.marketplaces} />
            </TabsContent>

            <TabsContent value="permissions" className="space-y-4">
              <PermissionsTab
                permissions={settings.permissions}
                onChange={(permissions) => setSettings({ ...settings, permissions })}
              />
            </TabsContent>

            <TabsContent value="account" className="space-y-4">
              <AccountTab accountInfo={settings.accountInfo} />
            </TabsContent>

            <TabsContent value="preferences" className="space-y-4">
              <PreferencesTab
                preferences={settings.globalPreferences}
                onChange={(preferences) => setSettings({ ...settings, globalPreferences: preferences })}
              />
            </TabsContent>
          </Tabs>
        </div>
      </main>
    </div>
  );
}

function MCPServersTab({ servers, onChange }: { servers: MCPServer[], onChange: (servers: MCPServer[]) => void }) {
  const [newServerName, setNewServerName] = useState("");
  const [newServerCommand, setNewServerCommand] = useState("");

  const addServer = () => {
    if (!newServerName || !newServerCommand) return;

    onChange([
      ...servers,
      {
        name: newServerName,
        command: newServerCommand,
        args: [],
        enabled: true,
      }
    ]);

    setNewServerName("");
    setNewServerCommand("");
  };

  const toggleServer = (index: number) => {
    const updated = [...servers];
    updated[index].enabled = !updated[index].enabled;
    onChange(updated);
  };

  const removeServer = (index: number) => {
    onChange(servers.filter((_, i) => i !== index));
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>MCP Servers</CardTitle>
        <CardDescription>
          Manage Model Context Protocol servers that extend Claude's capabilities
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          {servers.map((server, index) => (
            <div key={index} className="flex items-center justify-between p-3 border rounded-lg">
              <div className="flex-1">
                <div className="flex items-center gap-2">
                  <h4 className="font-medium">{server.name}</h4>
                  <Badge variant={server.enabled ? "default" : "secondary"}>
                    {server.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </div>
                <p className="text-sm text-muted-foreground mt-1">
                  <code className="text-xs">{server.command} {server.args.join(" ")}</code>
                </p>
              </div>
              <div className="flex gap-2">
                <Button variant="outline" size="sm" onClick={() => toggleServer(index)}>
                  {server.enabled ? "Disable" : "Enable"}
                </Button>
                <Button variant="destructive" size="sm" onClick={() => removeServer(index)}>
                  Remove
                </Button>
              </div>
            </div>
          ))}
        </div>

        <div className="border-t pt-4">
          <h4 className="font-medium mb-3">Add New MCP Server</h4>
          <div className="grid grid-cols-2 gap-3">
            <Input
              placeholder="Server name (e.g., filesystem)"
              value={newServerName}
              onChange={(e) => setNewServerName(e.target.value)}
            />
            <Input
              placeholder="Command (e.g., npx -y @modelcontextprotocol/server-filesystem)"
              value={newServerCommand}
              onChange={(e) => setNewServerCommand(e.target.value)}
            />
          </div>
          <Button className="mt-3" onClick={addServer}>Add Server</Button>
        </div>
      </CardContent>
    </Card>
  );
}

function SkillsTab({ skills }: { skills: Skill[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Installed Skills</CardTitle>
        <CardDescription>
          Skills extend Claude with specialized knowledge and workflows
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-2">
          {skills.map((skill, index) => (
            <div key={index} className="p-3 border rounded-lg">
              <div className="flex items-center justify-between">
                <div>
                  <div className="flex items-center gap-2">
                    <h4 className="font-medium">{skill.name}</h4>
                    <Badge variant={skill.type === "user" ? "default" : "secondary"}>
                      {skill.type}
                    </Badge>
                  </div>
                  <p className="text-sm text-muted-foreground mt-1">{skill.description}</p>
                  <p className="text-xs text-muted-foreground mt-1">
                    <code>{skill.location}</code>
                  </p>
                </div>
              </div>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

function PluginsTab({ plugins, onChange }: { plugins: Plugin[], onChange: (plugins: Plugin[]) => void }) {
  const togglePlugin = (index: number) => {
    const updated = [...plugins];
    updated[index].enabled = !updated[index].enabled;
    onChange(updated);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Plugins</CardTitle>
        <CardDescription>
          Manage installed plugins from the Claude Code marketplace
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-2">
          {plugins.map((plugin, index) => (
            <div key={index} className="flex items-center justify-between p-3 border rounded-lg">
              <div>
                <div className="flex items-center gap-2">
                  <h4 className="font-medium">{plugin.name}</h4>
                  <Badge variant="outline">{plugin.version}</Badge>
                  <Badge variant={plugin.enabled ? "default" : "secondary"}>
                    {plugin.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </div>
                <p className="text-sm text-muted-foreground mt-1">{plugin.description}</p>
              </div>
              <Button variant="outline" size="sm" onClick={() => togglePlugin(index)}>
                {plugin.enabled ? "Disable" : "Enable"}
              </Button>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

function PermissionsTab({ permissions, onChange }: { permissions: PermissionInfo, onChange: (permissions: PermissionInfo) => void }) {
  const [newCommand, setNewCommand] = useState("");

  const addAllowedCommand = () => {
    if (!newCommand) return;
    onChange({
      ...permissions,
      allowedCommands: [...permissions.allowedCommands, newCommand],
    });
    setNewCommand("");
  };

  const removeAllowedCommand = (index: number) => {
    onChange({
      ...permissions,
      allowedCommands: permissions.allowedCommands.filter((_, i) => i !== index),
    });
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Permissions & Security</CardTitle>
        <CardDescription>
          Configure command permissions and security settings
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <div>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={permissions.autoApproveEdits}
              onChange={(e) => onChange({ ...permissions, autoApproveEdits: e.target.checked })}
              className="rounded"
            />
            <span className="font-medium">Auto-approve file edits</span>
          </label>
          <p className="text-sm text-muted-foreground mt-1 ml-6">
            Automatically approve Read, Write, and Edit operations
          </p>
        </div>

        <div>
          <h4 className="font-medium mb-2">Allowed Commands</h4>
          <div className="space-y-2 mb-3">
            {permissions.allowedCommands.map((cmd, index) => (
              <div key={index} className="flex items-center justify-between p-2 bg-muted rounded">
                <code className="text-sm">{cmd}</code>
                <Button variant="ghost" size="sm" onClick={() => removeAllowedCommand(index)}>
                  Remove
                </Button>
              </div>
            ))}
          </div>
          <div className="flex gap-2">
            <Input
              placeholder="Add command pattern..."
              value={newCommand}
              onChange={(e) => setNewCommand(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && addAllowedCommand()}
            />
            <Button onClick={addAllowedCommand}>Add</Button>
          </div>
        </div>

        <div>
          <h4 className="font-medium mb-2">Blocked Commands</h4>
          <div className="space-y-2">
            {permissions.blockedCommands.map((cmd, index) => (
              <div key={index} className="p-2 bg-destructive/10 rounded">
                <code className="text-sm text-destructive">{cmd}</code>
              </div>
            ))}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

function AccountTab({ accountInfo }: { accountInfo: AccountInfo }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Account Information</CardTitle>
        <CardDescription>
          View your Claude API configuration and account details
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="text-sm font-medium text-muted-foreground">API Key Status</label>
            <div className="mt-1">
              <Badge variant={accountInfo.apiKeyConfigured ? "default" : "destructive"}>
                {accountInfo.apiKeyConfigured ? "Configured" : "Not Configured"}
              </Badge>
            </div>
          </div>
          <div>
            <label className="text-sm font-medium text-muted-foreground">Default Model</label>
            <p className="mt-1 font-medium">{accountInfo.model}</p>
          </div>
          {accountInfo.organization && (
            <div>
              <label className="text-sm font-medium text-muted-foreground">Organization</label>
              <p className="mt-1 font-medium">{accountInfo.organization}</p>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function PreferencesTab({ preferences, onChange }: { preferences: GlobalPreferences, onChange: (preferences: GlobalPreferences) => void }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Global Preferences</CardTitle>
        <CardDescription>
          Configure default behavior for Claude Code
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div>
          <label className="block text-sm font-medium mb-2">Theme</label>
          <select
            className="w-full p-2 border rounded"
            value={preferences.theme}
            onChange={(e) => onChange({ ...preferences, theme: e.target.value })}
          >
            <option value="auto">Auto (System)</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>

        <div>
          <label className="block text-sm font-medium mb-2">Output Format</label>
          <select
            className="w-full p-2 border rounded"
            value={preferences.outputFormat}
            onChange={(e) => onChange({ ...preferences, outputFormat: e.target.value })}
          >
            <option value="text">Text</option>
            <option value="json">JSON</option>
            <option value="stream-json">Stream JSON</option>
          </select>
        </div>

        <div>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={preferences.verbose}
              onChange={(e) => onChange({ ...preferences, verbose: e.target.checked })}
              className="rounded"
            />
            <span className="font-medium">Verbose Mode</span>
          </label>
          <p className="text-sm text-muted-foreground mt-1 ml-6">
            Show detailed logging and debug information
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

function MarketplacesTab({ marketplaces }: { marketplaces: Marketplace[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Plugin Marketplaces</CardTitle>
        <CardDescription>
          View available plugin marketplaces and their sources
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-3">
          {marketplaces.map((marketplace, index) => (
            <div key={index} className="p-4 border rounded-lg">
              <div className="flex items-start justify-between">
                <div className="flex-1">
                  <div className="flex items-center gap-2 mb-2">
                    <Store className="h-4 w-4 text-muted-foreground" />
                    <h4 className="font-medium">{marketplace.name}</h4>
                  </div>
                  <div className="space-y-1 text-sm text-muted-foreground">
                    <p className="flex items-center gap-2">
                      <span className="font-medium">Repository:</span>
                      <code className="text-xs bg-muted px-2 py-0.5 rounded">{marketplace.repo}</code>
                    </p>
                    <p className="flex items-center gap-2">
                      <span className="font-medium">Install Location:</span>
                      <code className="text-xs bg-muted px-2 py-0.5 rounded break-all">
                        {marketplace.installLocation}
                      </code>
                    </p>
                    <p className="flex items-center gap-2">
                      <span className="font-medium">Last Updated:</span>
                      <span className="text-xs">
                        {new Date(marketplace.lastUpdated).toLocaleString()}
                      </span>
                    </p>
                  </div>
                </div>
              </div>
            </div>
          ))}
          {marketplaces.length === 0 && (
            <div className="text-center py-8 text-muted-foreground">
              No marketplaces configured
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
