"use client";

import { useState, useEffect } from "react";
import { Header } from "@/components/layout/header";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { DeviceList } from "@/components/settings/device-list";
import { PairingDisplay } from "@/components/settings/pairing-display";
import { ScopeDirectory } from "@/components/settings/scope-directory";
import { ManagerSettings } from "@/components/settings/manager-settings";
import { Package, Puzzle, Shield, User, RefreshCw, Save, AlertCircle, Settings, Store, Loader2, Bot } from "lucide-react";

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

export default function SettingsPage() {
  const [claudeSettings, setClaudeSettings] = useState<ClaudeSettings | null>(null);
  const [claudeLoading, setClaudeLoading] = useState(true);
  const [claudeError, setClaudeError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetchClaudeSettings();
  }, []);

  const fetchClaudeSettings = async () => {
    try {
      setClaudeLoading(true);
      setClaudeError(null);
      const response = await fetch("/api/claude-settings");
      if (!response.ok) {
        throw new Error("Failed to fetch Claude Code settings");
      }
      const data = await response.json();
      setClaudeSettings(data);
    } catch (err) {
      setClaudeError(err instanceof Error ? err.message : "Unknown error");
    } finally {
      setClaudeLoading(false);
    }
  };

  const handleSaveClaudeSettings = async () => {
    if (!claudeSettings) return;

    try {
      setSaving(true);
      const response = await fetch("/api/claude-settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(claudeSettings),
      });

      if (!response.ok) {
        throw new Error("Failed to save settings");
      }

      await fetchClaudeSettings();
    } catch (err) {
      setClaudeError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Settings"
        description="Configure app settings and Claude Code"
      />
      <div className="flex-1 p-4 md:p-6 overflow-auto">
        <Tabs defaultValue="general" className="space-y-6">
          <TabsList className="w-full sm:w-auto">
            <TabsTrigger value="general" className="flex-1 sm:flex-initial">
              General
            </TabsTrigger>
            <TabsTrigger value="devices" className="flex-1 sm:flex-initial">
              Devices
            </TabsTrigger>
            <TabsTrigger value="manager" className="flex-1 sm:flex-initial">
              Manager
            </TabsTrigger>
            <TabsTrigger value="claude" className="flex-1 sm:flex-initial">
              Claude Code
            </TabsTrigger>
          </TabsList>

          <TabsContent value="general" className="space-y-6">
            <ScopeDirectory />
          </TabsContent>

          <TabsContent value="devices" className="space-y-6">
            <PairingDisplay />
            <DeviceList />
          </TabsContent>

          <TabsContent value="manager" className="space-y-6">
            <ManagerSettings />
          </TabsContent>

          <TabsContent value="claude" className="space-y-6">
            {claudeLoading ? (
              <div className="flex items-center justify-center py-12">
                <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
                <span className="ml-2 text-muted-foreground">Loading Claude settings...</span>
              </div>
            ) : claudeError ? (
              <Card className="border-destructive">
                <CardHeader>
                  <CardTitle className="flex items-center gap-2 text-destructive">
                    <AlertCircle className="h-5 w-5" />
                    Error Loading Settings
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  <p className="text-sm text-muted-foreground mb-4">{claudeError}</p>
                  <Button onClick={fetchClaudeSettings}>Retry</Button>
                </CardContent>
              </Card>
            ) : claudeSettings ? (
              <div className="space-y-6">
                <div className="flex items-center justify-between">
                  <div>
                    <h3 className="text-lg font-semibold">Claude Code Configuration</h3>
                    <p className="text-sm text-muted-foreground">
                      Manage MCP servers, skills, and permissions
                    </p>
                  </div>
                  <div className="flex gap-2">
                    <Button variant="outline" size="sm" onClick={fetchClaudeSettings}>
                      <RefreshCw className="h-4 w-4 mr-2" />
                      Refresh
                    </Button>
                    <Button size="sm" onClick={handleSaveClaudeSettings} disabled={saving}>
                      <Save className="h-4 w-4 mr-2" />
                      {saving ? "Saving..." : "Save"}
                    </Button>
                  </div>
                </div>

                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-sm font-medium text-muted-foreground">
                      Config Location
                    </CardTitle>
                  </CardHeader>
                  <CardContent>
                    <code className="text-xs bg-muted px-2 py-1 rounded">
                      {claudeSettings.configPath || "~/.claude/settings.json"}
                    </code>
                  </CardContent>
                </Card>

                <Tabs defaultValue="mcp" className="space-y-4">
                  <TabsList className="flex flex-wrap">
                    <TabsTrigger value="mcp">
                      <Package className="h-4 w-4 mr-2" />
                      MCP
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
                      Markets
                    </TabsTrigger>
                    <TabsTrigger value="permissions">
                      <Shield className="h-4 w-4 mr-2" />
                      Perms
                    </TabsTrigger>
                    <TabsTrigger value="account">
                      <User className="h-4 w-4 mr-2" />
                      Account
                    </TabsTrigger>
                  </TabsList>

                  <TabsContent value="mcp" className="space-y-4">
                    <MCPServersSection
                      servers={claudeSettings.mcpServers}
                      onChange={(servers) => setClaudeSettings({ ...claudeSettings, mcpServers: servers })}
                    />
                  </TabsContent>

                  <TabsContent value="skills" className="space-y-4">
                    <SkillsSection skills={claudeSettings.skills} />
                  </TabsContent>

                  <TabsContent value="plugins" className="space-y-4">
                    <PluginsSection
                      plugins={claudeSettings.plugins}
                      onChange={(plugins) => setClaudeSettings({ ...claudeSettings, plugins })}
                    />
                  </TabsContent>

                  <TabsContent value="marketplaces" className="space-y-4">
                    <MarketplacesSection marketplaces={claudeSettings.marketplaces} />
                  </TabsContent>

                  <TabsContent value="permissions" className="space-y-4">
                    <PermissionsSection
                      permissions={claudeSettings.permissions}
                      onChange={(permissions) => setClaudeSettings({ ...claudeSettings, permissions })}
                    />
                  </TabsContent>

                  <TabsContent value="account" className="space-y-4">
                    <AccountSection
                      accountInfo={claudeSettings.accountInfo}
                      preferences={claudeSettings.globalPreferences}
                      onPreferencesChange={(preferences) => setClaudeSettings({ ...claudeSettings, globalPreferences: preferences })}
                    />
                  </TabsContent>
                </Tabs>
              </div>
            ) : null}
          </TabsContent>
        </Tabs>
      </div>
    </div>
  );
}

function MCPServersSection({ servers, onChange }: { servers: MCPServer[], onChange: (servers: MCPServer[]) => void }) {
  const [newServerName, setNewServerName] = useState("");
  const [newServerCommand, setNewServerCommand] = useState("");

  const addServer = () => {
    if (!newServerName || !newServerCommand) return;
    onChange([
      ...servers,
      { name: newServerName, command: newServerCommand, args: [], enabled: true }
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
        <CardDescription>Model Context Protocol servers that extend Claude</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          {servers.map((server, index) => (
            <div key={index} className="flex items-center justify-between p-3 border rounded-lg">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <h4 className="font-medium">{server.name}</h4>
                  <Badge variant={server.enabled ? "default" : "secondary"}>
                    {server.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </div>
                <code className="text-xs text-muted-foreground truncate block mt-1">
                  {server.command} {server.args.join(" ")}
                </code>
              </div>
              <div className="flex gap-2 ml-2">
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
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <Input
              placeholder="Server name"
              value={newServerName}
              onChange={(e) => setNewServerName(e.target.value)}
            />
            <Input
              placeholder="Command"
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

function SkillsSection({ skills }: { skills: Skill[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Installed Skills</CardTitle>
        <CardDescription>Skills extend Claude with specialized knowledge</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-2">
          {skills.map((skill, index) => (
            <div key={index} className="p-3 border rounded-lg">
              <div className="flex items-center gap-2">
                <h4 className="font-medium">{skill.name}</h4>
                <Badge variant={skill.type === "user" ? "default" : "secondary"}>{skill.type}</Badge>
              </div>
              <p className="text-sm text-muted-foreground mt-1">{skill.description}</p>
              <code className="text-xs text-muted-foreground mt-1 block">{skill.location}</code>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

function PluginsSection({ plugins, onChange }: { plugins: Plugin[], onChange: (plugins: Plugin[]) => void }) {
  const togglePlugin = (index: number) => {
    const updated = [...plugins];
    updated[index].enabled = !updated[index].enabled;
    onChange(updated);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Claude Code Plugins</CardTitle>
        <CardDescription>Plugins from the Claude Code marketplace</CardDescription>
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

function MarketplacesSection({ marketplaces }: { marketplaces: Marketplace[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Plugin Marketplaces</CardTitle>
        <CardDescription>Available plugin sources</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-3">
          {marketplaces.map((marketplace, index) => (
            <div key={index} className="p-3 border rounded-lg">
              <div className="flex items-center gap-2 mb-2">
                <Store className="h-4 w-4 text-muted-foreground" />
                <h4 className="font-medium">{marketplace.name}</h4>
              </div>
              <div className="space-y-1 text-sm text-muted-foreground">
                <p><span className="font-medium">Repo:</span> <code className="text-xs">{marketplace.repo}</code></p>
                <p><span className="font-medium">Location:</span> <code className="text-xs">{marketplace.installLocation}</code></p>
                <p><span className="font-medium">Updated:</span> {new Date(marketplace.lastUpdated).toLocaleString()}</p>
              </div>
            </div>
          ))}
          {marketplaces.length === 0 && (
            <p className="text-center py-4 text-muted-foreground">No marketplaces configured</p>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function PermissionsSection({ permissions, onChange }: { permissions: PermissionInfo, onChange: (permissions: PermissionInfo) => void }) {
  const [newCommand, setNewCommand] = useState("");

  const addAllowedCommand = () => {
    if (!newCommand) return;
    onChange({ ...permissions, allowedCommands: [...permissions.allowedCommands, newCommand] });
    setNewCommand("");
  };

  const removeAllowedCommand = (index: number) => {
    onChange({ ...permissions, allowedCommands: permissions.allowedCommands.filter((_, i) => i !== index) });
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Permissions & Security</CardTitle>
        <CardDescription>Command permissions and security settings</CardDescription>
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
                <Button variant="ghost" size="sm" onClick={() => removeAllowedCommand(index)}>Remove</Button>
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

function AccountSection({ accountInfo, preferences, onPreferencesChange }: {
  accountInfo: AccountInfo;
  preferences: GlobalPreferences;
  onPreferencesChange: (preferences: GlobalPreferences) => void;
}) {
  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Account Information</CardTitle>
          <CardDescription>Claude API configuration</CardDescription>
        </CardHeader>
        <CardContent>
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

      <Card>
        <CardHeader>
          <CardTitle>Preferences</CardTitle>
          <CardDescription>Default behavior settings</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div>
            <label className="block text-sm font-medium mb-2">Theme</label>
            <select
              className="w-full p-2 border rounded"
              value={preferences.theme}
              onChange={(e) => onPreferencesChange({ ...preferences, theme: e.target.value })}
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
              onChange={(e) => onPreferencesChange({ ...preferences, outputFormat: e.target.value })}
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
                onChange={(e) => onPreferencesChange({ ...preferences, verbose: e.target.checked })}
                className="rounded"
              />
              <span className="font-medium">Verbose Mode</span>
            </label>
            <p className="text-sm text-muted-foreground mt-1 ml-6">Show detailed logging</p>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
