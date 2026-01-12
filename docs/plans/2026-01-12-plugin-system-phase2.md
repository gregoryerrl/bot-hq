# Plugin System Phase 2 - UI & Integration

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the Plugins tab UI, Create Task dialog, and approval dialog with plugin action checkboxes.

**Architecture:** React components fetch plugin data from Phase 1 APIs. Approval dialog queries plugin registry for available actions and renders them as checkboxes. Create Task dialog uses manager bot for task refinement.

**Tech Stack:** Next.js 16 App Router, React, Tailwind CSS, shadcn/ui components, Lucide icons

---

## Task 1: Add Checkbox UI Component

**Files:**
- Create: `src/components/ui/checkbox.tsx`

**Step 1: Create checkbox component**

```tsx
"use client";

import * as React from "react";
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { Check } from "lucide-react";
import { cn } from "@/lib/utils";

const Checkbox = React.forwardRef<
  React.ElementRef<typeof CheckboxPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>
>(({ className, ...props }, ref) => (
  <CheckboxPrimitive.Root
    ref={ref}
    className={cn(
      "peer h-4 w-4 shrink-0 rounded-sm border border-primary ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground",
      className
    )}
    {...props}
  >
    <CheckboxPrimitive.Indicator
      className={cn("flex items-center justify-center text-current")}
    >
      <Check className="h-3 w-3" />
    </CheckboxPrimitive.Indicator>
  </CheckboxPrimitive.Root>
));
Checkbox.displayName = CheckboxPrimitive.Root.displayName;

export { Checkbox };
```

**Step 2: Install radix checkbox dependency**

Run: `npm install @radix-ui/react-checkbox`
Expected: Package added to package.json

**Step 3: Verify component compiles**

Run: `npm run build 2>&1 | head -50`
Expected: No errors mentioning checkbox

**Step 4: Commit**

```bash
git add src/components/ui/checkbox.tsx package.json package-lock.json
git commit -m "feat: add Checkbox UI component"
```

---

## Task 2: Create Plugins Page Route

**Files:**
- Create: `src/app/plugins/page.tsx`

**Step 1: Create plugins page**

```tsx
import { Header } from "@/components/layout/header";
import { PluginList } from "@/components/plugins/plugin-list";

export default function PluginsPage() {
  return (
    <div className="flex flex-col h-full">
      <Header
        title="Plugins"
        description="Manage installed plugins and their settings"
      />
      <div className="flex-1 p-4 md:p-6">
        <PluginList />
      </div>
    </div>
  );
}
```

**Step 2: Verify file exists**

Run: `ls -la src/app/plugins/`
Expected: page.tsx file exists

**Step 3: Commit**

```bash
git add src/app/plugins/page.tsx
git commit -m "feat: add plugins page route"
```

---

## Task 3: Create Plugin Card Component

**Files:**
- Create: `src/components/plugins/plugin-card.tsx`

**Step 1: Create plugin card**

```tsx
"use client";

import { useState } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Settings, Power, Puzzle } from "lucide-react";

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
```

**Step 2: Verify file exists**

Run: `ls -la src/components/plugins/`
Expected: plugin-card.tsx file exists

**Step 3: Commit**

```bash
git add src/components/plugins/plugin-card.tsx
git commit -m "feat: add PluginCard component"
```

---

## Task 4: Create Switch UI Component

**Files:**
- Create: `src/components/ui/switch.tsx`

**Step 1: Create switch component**

```tsx
"use client";

import * as React from "react";
import * as SwitchPrimitive from "@radix-ui/react-switch";
import { cn } from "@/lib/utils";

const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>
>(({ className, ...props }, ref) => (
  <SwitchPrimitive.Root
    className={cn(
      "peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-input",
      className
    )}
    {...props}
    ref={ref}
  >
    <SwitchPrimitive.Thumb
      className={cn(
        "pointer-events-none block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0"
      )}
    />
  </SwitchPrimitive.Root>
));
Switch.displayName = SwitchPrimitive.Root.displayName;

export { Switch };
```

**Step 2: Install radix switch dependency**

Run: `npm install @radix-ui/react-switch`
Expected: Package added to package.json

**Step 3: Verify component compiles**

Run: `npm run build 2>&1 | head -50`
Expected: No errors mentioning switch

**Step 4: Commit**

```bash
git add src/components/ui/switch.tsx package.json package-lock.json
git commit -m "feat: add Switch UI component"
```

---

## Task 5: Create Plugin Settings Dialog

**Files:**
- Create: `src/components/plugins/plugin-settings-dialog.tsx`

**Step 1: Create settings dialog**

```tsx
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
```

**Step 2: Create Label UI component**

```tsx
// src/components/ui/label.tsx
"use client";

import * as React from "react";
import * as LabelPrimitive from "@radix-ui/react-label";
import { cn } from "@/lib/utils";

const Label = React.forwardRef<
  React.ElementRef<typeof LabelPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof LabelPrimitive.Root>
>(({ className, ...props }, ref) => (
  <LabelPrimitive.Root
    ref={ref}
    className={cn(
      "text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
      className
    )}
    {...props}
  />
));
Label.displayName = LabelPrimitive.Root.displayName;

export { Label };
```

**Step 3: Install radix label dependency**

Run: `npm install @radix-ui/react-label`
Expected: Package added to package.json

**Step 4: Verify files exist**

Run: `ls -la src/components/plugins/ src/components/ui/label.tsx`
Expected: Both files exist

**Step 5: Commit**

```bash
git add src/components/plugins/plugin-settings-dialog.tsx src/components/ui/label.tsx package.json package-lock.json
git commit -m "feat: add PluginSettingsDialog and Label component"
```

---

## Task 6: Create Plugin List Component

**Files:**
- Create: `src/components/plugins/plugin-list.tsx`

**Step 1: Create plugin list**

```tsx
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
```

**Step 2: Verify file exists**

Run: `ls -la src/components/plugins/`
Expected: plugin-list.tsx, plugin-card.tsx, plugin-settings-dialog.tsx

**Step 3: Commit**

```bash
git add src/components/plugins/plugin-list.tsx
git commit -m "feat: add PluginList component"
```

---

## Task 7: Add Plugins Tab to Sidebar

**Files:**
- Modify: `src/components/layout/sidebar.tsx:6-17`

**Step 1: Add Puzzle icon import and plugins nav item**

Update the imports and navItems array:

```tsx
import { LayoutDashboard, Clock, ScrollText, Settings, Globe, MessageSquare, Terminal, FileText, Puzzle } from "lucide-react";

const navItems = [
  { href: "/", label: "Taskboard", icon: LayoutDashboard },
  { href: "/pending", label: "Pending", icon: Clock },
  { href: "/chat", label: "Claude Chat", icon: MessageSquare },
  { href: "/terminal", label: "Terminal", icon: Terminal },
  { href: "/docs", label: "Docs", icon: FileText },
  { href: "/logs", label: "Logs", icon: ScrollText },
  { href: "/plugins", label: "Plugins", icon: Puzzle },
  { href: "/settings", label: "Settings", icon: Settings },
  { href: "/claude-settings", label: "Claude Settings", icon: Globe },
];
```

**Step 2: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/components/layout/sidebar.tsx
git commit -m "feat: add Plugins tab to sidebar"
```

---

## Task 8: Create Plugin Actions API Route

**Files:**
- Create: `src/app/api/plugins/actions/route.ts`

**Step 1: Create actions API**

```tsx
// src/app/api/plugins/actions/route.ts

import { NextRequest, NextResponse } from "next/server";
import { getPluginEvents } from "@/lib/plugins";

export async function GET(request: NextRequest) {
  try {
    const searchParams = request.nextUrl.searchParams;
    const type = searchParams.get("type"); // "approval" | "task" | "workspace"

    const events = getPluginEvents();
    let actions;

    switch (type) {
      case "approval":
        actions = await events.getApprovalActions();
        break;
      case "task":
        actions = await events.getTaskActions();
        break;
      default:
        return NextResponse.json(
          { error: "Invalid action type. Use 'approval', 'task', or 'workspace'" },
          { status: 400 }
        );
    }

    return NextResponse.json({
      actions: actions.map(a => ({
        pluginName: a.pluginName,
        id: a.action.id,
        label: a.action.label,
        description: typeof a.action.description === "string"
          ? a.action.description
          : undefined,
        icon: a.action.icon,
        defaultChecked: a.action.defaultChecked ?? false,
      })),
    });
  } catch (error) {
    console.error("Failed to get plugin actions:", error);
    return NextResponse.json(
      { error: "Failed to get plugin actions" },
      { status: 500 }
    );
  }
}
```

**Step 2: Verify file exists**

Run: `ls -la src/app/api/plugins/actions/`
Expected: route.ts exists

**Step 3: Commit**

```bash
git add src/app/api/plugins/actions/route.ts
git commit -m "feat: add plugin actions API endpoint"
```

---

## Task 9: Create Plugin Execute Action API

**Files:**
- Create: `src/app/api/plugins/actions/execute/route.ts`

**Step 1: Create execute action API**

```tsx
// src/app/api/plugins/actions/execute/route.ts

import { NextRequest, NextResponse } from "next/server";
import { getPluginEvents, getPluginRegistry, createPluginContext } from "@/lib/plugins";
import { db, tasks, approvals, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { pluginName, actionId, approvalId } = body;

    if (!pluginName || !actionId || !approvalId) {
      return NextResponse.json(
        { error: "Missing required fields: pluginName, actionId, approvalId" },
        { status: 400 }
      );
    }

    // Get approval details
    const approval = await db.query.approvals.findFirst({
      where: eq(approvals.id, approvalId),
    });

    if (!approval) {
      return NextResponse.json(
        { error: "Approval not found" },
        { status: 404 }
      );
    }

    // Get task and workspace
    const task = await db.query.tasks.findFirst({
      where: eq(tasks.id, approval.taskId),
    });

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, approval.workspaceId),
    });

    if (!task || !workspace) {
      return NextResponse.json(
        { error: "Task or workspace not found" },
        { status: 404 }
      );
    }

    // Get plugin and its actions
    const registry = getPluginRegistry();
    const plugin = registry.getPlugin(pluginName);

    if (!plugin || !plugin.enabled) {
      return NextResponse.json(
        { error: "Plugin not found or disabled" },
        { status: 404 }
      );
    }

    // Find the action
    const events = getPluginEvents();
    const approvalActions = await events.getApprovalActions();
    const actionDef = approvalActions.find(
      a => a.pluginName === pluginName && a.action.id === actionId
    );

    if (!actionDef) {
      return NextResponse.json(
        { error: "Action not found" },
        { status: 404 }
      );
    }

    // Create context and execute action
    const context = await createPluginContext(plugin);
    const actionContext = {
      approval: {
        id: approval.id,
        branchName: approval.branchName,
        baseBranch: approval.baseBranch,
        commitMessages: approval.commitMessages ? JSON.parse(approval.commitMessages) : [],
        diffSummary: approval.diffSummary ? JSON.parse(approval.diffSummary) : null,
      },
      task: {
        id: task.id,
        title: task.title,
        description: task.description,
        state: task.state,
      },
      workspace: {
        id: workspace.id,
        name: workspace.name,
        repoPath: workspace.repoPath,
      },
      pluginContext: context,
    };

    const result = await actionDef.action.handler(actionContext);

    return NextResponse.json({
      success: result.success,
      message: result.message,
      data: result.data,
    });
  } catch (error) {
    console.error("Failed to execute plugin action:", error);
    return NextResponse.json(
      { error: `Failed to execute action: ${error}` },
      { status: 500 }
    );
  }
}
```

**Step 2: Verify file exists**

Run: `ls -la src/app/api/plugins/actions/`
Expected: route.ts and execute/route.ts exist

**Step 3: Commit**

```bash
git add src/app/api/plugins/actions/execute/route.ts
git commit -m "feat: add plugin action execution API"
```

---

## Task 10: Create Plugin Action Checkboxes Component

**Files:**
- Create: `src/components/plugins/plugin-action-checkboxes.tsx`

**Step 1: Create action checkboxes component**

```tsx
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

  useEffect(() => {
    fetchActions();
  }, [type]);

  useEffect(() => {
    // Initialize with default checked actions
    if (actions.length > 0 && selectedActions.length === 0) {
      const defaults = actions
        .filter(a => a.defaultChecked)
        .map(a => `${a.pluginName}:${a.id}`);
      if (defaults.length > 0) {
        onSelectionChange(defaults);
      }
    }
  }, [actions]);

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
```

**Step 2: Verify file exists**

Run: `ls -la src/components/plugins/`
Expected: plugin-action-checkboxes.tsx exists

**Step 3: Commit**

```bash
git add src/components/plugins/plugin-action-checkboxes.tsx
git commit -m "feat: add PluginActionCheckboxes component"
```

---

## Task 11: Update Approval Dialog with Plugin Actions

**Files:**
- Modify: `src/components/pending-board/draft-pr-card.tsx`

**Step 1: Add imports for plugin actions**

Add at the top of the file (after existing imports):

```tsx
import { PluginActionCheckboxes } from "@/components/plugins/plugin-action-checkboxes";
```

**Step 2: Add state for selected plugin actions**

Add inside the component, after the existing useState calls:

```tsx
const [selectedPluginActions, setSelectedPluginActions] = useState<string[]>([]);
```

**Step 3: Update the Approve Dialog content**

Replace the existing Approve Dialog content (lines 228-268) with plugin action checkboxes:

Find and replace the Dialog section starting at line 228:

```tsx
{/* Approve Dialog */}
<Dialog open={showApproveDialog} onOpenChange={setShowApproveDialog}>
  <DialogContent className="max-w-lg">
    <DialogHeader>
      <DialogTitle>Accept Changes</DialogTitle>
    </DialogHeader>
    <div className="py-4 space-y-4">
      <p className="text-sm text-muted-foreground">
        This will keep the commits on branch{" "}
        <code className="bg-muted px-1 py-0.5 rounded">
          {approval.branchName}
        </code>
        .
      </p>

      <PluginActionCheckboxes
        type="approval"
        selectedActions={selectedPluginActions}
        onSelectionChange={setSelectedPluginActions}
      />

      <div className="space-y-2">
        <label className="text-sm font-medium">
          Documentation (optional)
        </label>
        <Textarea
          placeholder="e.g. &quot;Document the auth middleware&quot;"
          value={docRequest}
          onChange={(e) => setDocRequest(e.target.value)}
          rows={3}
        />
        <p className="text-xs text-muted-foreground">
          If provided, an agent will create documentation after acceptance.
        </p>
      </div>
    </div>
    <DialogFooter>
      <Button
        variant="outline"
        onClick={() => setShowApproveDialog(false)}
      >
        Cancel
      </Button>
      <Button
        onClick={handleApprove}
        disabled={loading}
        className="bg-green-600 hover:bg-green-700 text-white"
      >
        {loading ? "Accepting..." : "Accept"}
      </Button>
    </DialogFooter>
  </DialogContent>
</Dialog>
```

**Step 4: Update handleApprove to pass plugin actions**

Update the handleApprove function to include selected actions:

```tsx
const handleApprove = async () => {
  setLoading(true);
  await onApprove(approval.id, docRequest.trim() || undefined, selectedPluginActions);
  setShowApproveDialog(false);
  setDocRequest("");
  setSelectedPluginActions([]);
  setLoading(false);
};
```

**Step 5: Update interface to accept plugin actions**

Update DraftPRCardProps interface:

```tsx
interface DraftPRCardProps {
  approval: Approval & {
    taskTitle?: string;
    workspaceName?: string;
    githubIssueNumber?: number;
  };
  onApprove: (id: number, docRequest?: string, pluginActions?: string[]) => void;
  onReject: (id: number) => void;
  onRequestChanges: (id: number, instructions: string) => void;
}
```

**Step 6: Update button text**

Change "Approve & Create PR" to just "Accept" since PR creation is now a plugin action:

Find the approve button (around line 215-223) and update:

```tsx
<Button
  size="sm"
  className="bg-green-600 hover:bg-green-700 text-white"
  onClick={() => setShowApproveDialog(true)}
>
  <Check className="h-4 w-4 mr-1" />
  Accept
</Button>
```

Also change "Reject" to "Decline":

```tsx
<Button
  size="sm"
  variant="outline"
  className="text-red-600 hover:text-red-700 hover:bg-red-50"
  onClick={() => onReject(approval.id)}
>
  <X className="h-4 w-4 mr-1" />
  Decline
</Button>
```

**Step 7: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 8: Commit**

```bash
git add src/components/pending-board/draft-pr-card.tsx
git commit -m "feat: add plugin action checkboxes to approval dialog"
```

---

## Task 12: Update Approval API to Execute Plugin Actions

**Files:**
- Modify: `src/app/api/approvals/[id]/route.ts`

**Step 1: Update handleApprove to accept and execute plugin actions**

Add import at the top:

```tsx
import { fireApprovalAccepted } from "@/lib/plugins";
```

**Step 2: Update POST handler to accept pluginActions**

Update the body parsing:

```tsx
const { action, instructions, docRequest, pluginActions } = await request.json();
```

**Step 3: Update handleApprove signature and add plugin execution**

Update the handleApprove function to accept pluginActions and execute them:

```tsx
async function handleApprove(
  approval: typeof approvals.$inferSelect,
  task: typeof tasks.$inferSelect,
  workspace: typeof workspaces.$inferSelect,
  repoPath: string,
  docRequest?: string,
  pluginActions?: string[]
) {
  // Update approval status
  await db
    .update(approvals)
    .set({ status: "approved", resolvedAt: new Date() })
    .where(eq(approvals.id, approval.id));

  // Update task state - just mark as done (no PR creation in core)
  await db
    .update(tasks)
    .set({
      state: "done",
      updatedAt: new Date(),
    })
    .where(eq(tasks.id, task.id));

  // Log the action
  await db.insert(logs).values({
    workspaceId: workspace.id,
    taskId: task.id,
    type: "approval",
    message: `Changes accepted. Branch ${approval.branchName} kept locally.`,
  });

  // Fire hook for plugins
  await fireApprovalAccepted(
    {
      id: approval.id,
      branchName: approval.branchName,
      baseBranch: approval.baseBranch,
      commitMessages: approval.commitMessages ? JSON.parse(approval.commitMessages) : [],
    },
    {
      id: task.id,
      title: task.title,
      description: task.description,
    }
  );

  // Execute selected plugin actions
  if (pluginActions && pluginActions.length > 0) {
    for (const actionKey of pluginActions) {
      const [pluginName, actionId] = actionKey.split(":");
      try {
        const res = await fetch(`${process.env.NEXT_PUBLIC_BASE_URL || 'http://localhost:3000'}/api/plugins/actions/execute`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            pluginName,
            actionId,
            approvalId: approval.id,
          }),
        });

        if (!res.ok) {
          const error = await res.json();
          console.error(`Plugin action ${actionKey} failed:`, error);
          await db.insert(logs).values({
            workspaceId: workspace.id,
            taskId: task.id,
            type: "error",
            message: `Plugin action ${pluginName}:${actionId} failed: ${error.error}`,
          });
        } else {
          await db.insert(logs).values({
            workspaceId: workspace.id,
            taskId: task.id,
            type: "approval",
            message: `Plugin action ${pluginName}:${actionId} executed successfully`,
          });
        }
      } catch (error) {
        console.error(`Plugin action ${actionKey} error:`, error);
      }
    }
  }

  // If documentation was requested, spawn a follow-up task
  if (docRequest) {
    const docPrompt = `${docRequest}

Context:
- Title: ${task.title}
- Branch: ${approval.branchName}

Please write documentation to the agent-docs folder based on the request above.`;

    await startAgentForTask(task.id, docPrompt);

    await db.insert(logs).values({
      workspaceId: workspace.id,
      taskId: task.id,
      type: "agent",
      message: `Documentation task spawned: ${docRequest}`,
    });
  }
}
```

**Step 4: Update handleApprove call**

Update the call in the POST handler:

```tsx
if (action === "approve") {
  await handleApprove(approval, task, workspace, repoPath, docRequest, pluginActions);
}
```

**Step 5: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 6: Commit**

```bash
git add src/app/api/approvals/[id]/route.ts
git commit -m "feat: execute plugin actions on approval accept"
```

---

## Task 13: Update Approval List to Pass Plugin Actions

**Files:**
- Modify: `src/components/pending-board/approval-list.tsx`

**Step 1: Read the current approval-list.tsx**

Read the file to understand its current implementation.

**Step 2: Update onApprove callback signature**

Update the handleApprove function to accept and pass pluginActions:

```tsx
async function handleApprove(id: number, docRequest?: string, pluginActions?: string[]) {
  try {
    await fetch(`/api/approvals/${id}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ action: "approve", docRequest, pluginActions }),
    });
    fetchApprovals();
  } catch (error) {
    console.error("Failed to approve:", error);
  }
}
```

**Step 3: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add src/components/pending-board/approval-list.tsx
git commit -m "feat: pass plugin actions from approval list"
```

---

## Task 14: Create Task Dialog Component

**Files:**
- Create: `src/components/taskboard/create-task-dialog.tsx`

**Step 1: Create the dialog component**

```tsx
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
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Loader2 } from "lucide-react";

interface Workspace {
  id: number;
  name: string;
  repoPath: string;
}

interface CreateTaskDialogProps {
  open: boolean;
  onClose: () => void;
  onTaskCreated: () => void;
}

export function CreateTaskDialog({
  open,
  onClose,
  onTaskCreated,
}: CreateTaskDialogProps) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [selectedWorkspace, setSelectedWorkspace] = useState<string>("");
  const [prompt, setPrompt] = useState("");
  const [refinedTitle, setRefinedTitle] = useState("");
  const [refinedDescription, setRefinedDescription] = useState("");
  const [loading, setLoading] = useState(false);
  const [refining, setRefining] = useState(false);
  const [step, setStep] = useState<"input" | "preview">("input");

  useEffect(() => {
    if (open) {
      fetchWorkspaces();
      resetState();
    }
  }, [open]);

  const resetState = () => {
    setSelectedWorkspace("");
    setPrompt("");
    setRefinedTitle("");
    setRefinedDescription("");
    setStep("input");
    setLoading(false);
    setRefining(false);
  };

  const fetchWorkspaces = async () => {
    try {
      const res = await fetch("/api/workspaces");
      const data = await res.json();
      setWorkspaces(data);
    } catch (error) {
      console.error("Failed to fetch workspaces:", error);
    }
  };

  const handleRefine = async () => {
    if (!selectedWorkspace || !prompt.trim()) return;

    setRefining(true);
    try {
      const res = await fetch("/api/manager/summary", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          workspaceId: parseInt(selectedWorkspace),
          prompt: prompt.trim(),
        }),
      });

      if (res.ok) {
        const data = await res.json();
        setRefinedTitle(data.title || prompt.slice(0, 50));
        setRefinedDescription(data.description || prompt);
        setStep("preview");
      } else {
        // Fallback to using prompt as-is
        setRefinedTitle(prompt.slice(0, 100));
        setRefinedDescription(prompt);
        setStep("preview");
      }
    } catch (error) {
      console.error("Failed to refine task:", error);
      // Fallback to using prompt as-is
      setRefinedTitle(prompt.slice(0, 100));
      setRefinedDescription(prompt);
      setStep("preview");
    } finally {
      setRefining(false);
    }
  };

  const handleCreate = async () => {
    if (!selectedWorkspace) return;

    setLoading(true);
    try {
      await fetch("/api/tasks", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          workspaceId: parseInt(selectedWorkspace),
          title: refinedTitle,
          description: refinedDescription,
          state: "new",
        }),
      });
      onTaskCreated();
      onClose();
    } catch (error) {
      console.error("Failed to create task:", error);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {step === "input" ? "Create New Task" : "Review Task"}
          </DialogTitle>
        </DialogHeader>

        {step === "input" ? (
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label>Workspace</Label>
              <Select
                value={selectedWorkspace}
                onValueChange={setSelectedWorkspace}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select a workspace..." />
                </SelectTrigger>
                <SelectContent>
                  {workspaces.map((ws) => (
                    <SelectItem key={ws.id} value={ws.id.toString()}>
                      {ws.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>What do you want to build?</Label>
              <Textarea
                placeholder="Describe the feature, bug fix, or task..."
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                rows={4}
              />
              <p className="text-xs text-muted-foreground">
                A manager bot will analyze your codebase and refine this into a task.
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label>Title</Label>
              <Textarea
                value={refinedTitle}
                onChange={(e) => setRefinedTitle(e.target.value)}
                rows={2}
              />
            </div>

            <div className="space-y-2">
              <Label>Description</Label>
              <Textarea
                value={refinedDescription}
                onChange={(e) => setRefinedDescription(e.target.value)}
                rows={6}
              />
            </div>

            <p className="text-xs text-muted-foreground">
              Review and edit the task details before creating.
            </p>
          </div>
        )}

        <DialogFooter>
          {step === "input" ? (
            <>
              <Button variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button
                onClick={handleRefine}
                disabled={!selectedWorkspace || !prompt.trim() || refining}
              >
                {refining ? (
                  <>
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                    Refining...
                  </>
                ) : (
                  "Continue"
                )}
              </Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={() => setStep("input")}>
                Back
              </Button>
              <Button onClick={handleCreate} disabled={loading}>
                {loading ? (
                  <>
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                    Creating...
                  </>
                ) : (
                  "Create Task"
                )}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
```

**Step 2: Create Select UI component**

Create the Select component for workspace selection:

```tsx
// src/components/ui/select.tsx
"use client";

import * as React from "react";
import * as SelectPrimitive from "@radix-ui/react-select";
import { Check, ChevronDown, ChevronUp } from "lucide-react";
import { cn } from "@/lib/utils";

const Select = SelectPrimitive.Root;
const SelectGroup = SelectPrimitive.Group;
const SelectValue = SelectPrimitive.Value;

const SelectTrigger = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Trigger>
>(({ className, children, ...props }, ref) => (
  <SelectPrimitive.Trigger
    ref={ref}
    className={cn(
      "flex h-9 w-full items-center justify-between whitespace-nowrap rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm ring-offset-background placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1",
      className
    )}
    {...props}
  >
    {children}
    <SelectPrimitive.Icon asChild>
      <ChevronDown className="h-4 w-4 opacity-50" />
    </SelectPrimitive.Icon>
  </SelectPrimitive.Trigger>
));
SelectTrigger.displayName = SelectPrimitive.Trigger.displayName;

const SelectContent = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Content>
>(({ className, children, position = "popper", ...props }, ref) => (
  <SelectPrimitive.Portal>
    <SelectPrimitive.Content
      ref={ref}
      className={cn(
        "relative z-50 max-h-96 min-w-[8rem] overflow-hidden rounded-md border bg-popover text-popover-foreground shadow-md data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2",
        position === "popper" &&
          "data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1",
        className
      )}
      position={position}
      {...props}
    >
      <SelectPrimitive.Viewport
        className={cn(
          "p-1",
          position === "popper" &&
            "h-[var(--radix-select-trigger-height)] w-full min-w-[var(--radix-select-trigger-width)]"
        )}
      >
        {children}
      </SelectPrimitive.Viewport>
    </SelectPrimitive.Content>
  </SelectPrimitive.Portal>
));
SelectContent.displayName = SelectPrimitive.Content.displayName;

const SelectItem = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Item>
>(({ className, children, ...props }, ref) => (
  <SelectPrimitive.Item
    ref={ref}
    className={cn(
      "relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pl-2 pr-8 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
      className
    )}
    {...props}
  >
    <span className="absolute right-2 flex h-3.5 w-3.5 items-center justify-center">
      <SelectPrimitive.ItemIndicator>
        <Check className="h-4 w-4" />
      </SelectPrimitive.ItemIndicator>
    </span>
    <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
  </SelectPrimitive.Item>
));
SelectItem.displayName = SelectPrimitive.Item.displayName;

export {
  Select,
  SelectGroup,
  SelectValue,
  SelectTrigger,
  SelectContent,
  SelectItem,
};
```

**Step 3: Install radix select dependency**

Run: `npm install @radix-ui/react-select`
Expected: Package added to package.json

**Step 4: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 5: Commit**

```bash
git add src/components/taskboard/create-task-dialog.tsx src/components/ui/select.tsx package.json package-lock.json
git commit -m "feat: add CreateTaskDialog with workspace selection"
```

---

## Task 15: Add Create Task Button to Taskboard

**Files:**
- Modify: `src/app/page.tsx`

**Step 1: Update taskboard page with Create Task button**

```tsx
"use client";

import { useState } from "react";
import { Header } from "@/components/layout/header";
import { TaskList } from "@/components/taskboard/task-list";
import { SyncButton } from "@/components/taskboard/sync-button";
import { CreateTaskDialog } from "@/components/taskboard/create-task-dialog";
import { Button } from "@/components/ui/button";
import { Plus } from "lucide-react";

export default function TaskboardPage() {
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  const handleTaskCreated = () => {
    setRefreshKey(k => k + 1);
  };

  return (
    <div className="flex flex-col h-full">
      <Header
        title="Taskboard"
        description="Manage tasks across all workspaces"
      />
      <div className="flex-1 p-4 md:p-6">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-6">
          <div className="text-sm text-muted-foreground">
            Tasks from all workspaces
          </div>
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              onClick={() => setShowCreateDialog(true)}
            >
              <Plus className="h-4 w-4 mr-1" />
              Create Task
            </Button>
            <SyncButton />
          </div>
        </div>
        <TaskList key={refreshKey} />
      </div>

      <CreateTaskDialog
        open={showCreateDialog}
        onClose={() => setShowCreateDialog(false)}
        onTaskCreated={handleTaskCreated}
      />
    </div>
  );
}
```

**Step 2: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/app/page.tsx
git commit -m "feat: add Create Task button to taskboard"
```

---

## Task 16: Create Tasks POST API Route

**Files:**
- Modify: `src/app/api/tasks/route.ts`

**Step 1: Read current tasks route**

Read the file to understand current implementation.

**Step 2: Add POST handler for creating tasks**

Add a POST handler to the existing route:

```tsx
export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { workspaceId, title, description, state = "new" } = body;

    if (!workspaceId || !title) {
      return NextResponse.json(
        { error: "Missing required fields: workspaceId, title" },
        { status: 400 }
      );
    }

    const result = await db.insert(tasks).values({
      workspaceId,
      title,
      description: description || "",
      state,
      priority: 0,
      createdAt: new Date(),
      updatedAt: new Date(),
    }).returning();

    // Fire task created hook
    try {
      const { fireTaskCreated } = await import("@/lib/plugins");
      await fireTaskCreated({
        id: result[0].id,
        title: result[0].title,
        description: result[0].description,
      });
    } catch (e) {
      console.error("Failed to fire task created hook:", e);
    }

    return NextResponse.json(result[0]);
  } catch (error) {
    console.error("Failed to create task:", error);
    return NextResponse.json(
      { error: "Failed to create task" },
      { status: 500 }
    );
  }
}
```

**Step 3: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add src/app/api/tasks/route.ts
git commit -m "feat: add POST handler for creating tasks"
```

---

## Task 17: Update Empty State Messages

**Files:**
- Modify: `src/components/taskboard/task-list.tsx:64-69`

**Step 1: Update empty state message**

Change the empty state message to not reference GitHub:

```tsx
if (tasks.length === 0) {
  return (
    <div className="rounded-lg border border-dashed p-8 text-center text-muted-foreground">
      No tasks found. Click "Create Task" to add a new task.
    </div>
  );
}
```

**Step 2: Verify build passes**

Run: `npm run build 2>&1 | head -50`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add src/components/taskboard/task-list.tsx
git commit -m "refactor: update empty state message"
```

---

## Task 18: Final Build and Integration Test

**Files:** None (verification only)

**Step 1: Run full build**

Run: `npm run build`
Expected: Build succeeds with no errors

**Step 2: Start dev server**

Run: `npm run dev -p 7890`
Expected: Server starts on port 7890

**Step 3: Test plugins page**

Run: `curl -s http://localhost:7890/api/plugins | jq`
Expected: Returns list of plugins

**Step 4: Test plugin actions API**

Run: `curl -s "http://localhost:7890/api/plugins/actions?type=approval" | jq`
Expected: Returns empty actions array (no plugins with actions yet)

**Step 5: Verify pages load**

Test in browser:
- http://localhost:7890/plugins - Should show plugins page
- http://localhost:7890/ - Should show taskboard with Create Task button

**Step 6: Commit final verification**

```bash
git add -A
git commit -m "chore: Phase 2 complete - Plugins UI and approval integration"
```

---

## Summary

Phase 2 implements:

1. **Plugins Tab** - New page at `/plugins` to view and manage installed plugins
2. **Plugin Cards** - Display plugin info, enable/disable toggle, settings button
3. **Plugin Settings Dialog** - Configure plugin settings and credentials
4. **Create Task Dialog** - Manual task creation with workspace selection
5. **Plugin Action Checkboxes** - Show in approval dialog for Accept actions
6. **Updated Approval Flow** - Accept keeps commits, executes selected plugin actions
7. **API Endpoints** - `/api/plugins/actions` and `/api/plugins/actions/execute`

Next Phase (Phase 3) will:
- Remove GitHub integration from core
- Make bot-hq work completely standalone
- Prepare for GitHub plugin implementation
