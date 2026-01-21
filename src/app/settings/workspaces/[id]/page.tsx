"use client";

import { useState, useEffect, useCallback, use } from "react";
import { useRouter } from "next/navigation";
import { Header } from "@/components/layout/header";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { RuleListEditor } from "@/components/settings/rule-list-editor";
import { ArrowLeft, Save, RefreshCw } from "lucide-react";
import { AgentConfig, DEFAULT_AGENT_CONFIG } from "@/lib/agents/config-types";
import { toast } from "sonner";
import { GitRemoteSettings } from "@/components/git-remote/git-remote-settings";

export default function WorkspaceConfigPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const router = useRouter();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [savingContext, setSavingContext] = useState(false);
  const [workspaceName, setWorkspaceName] = useState("");
  const [repoPath, setRepoPath] = useState("");
  const [config, setConfig] = useState<AgentConfig>(DEFAULT_AGENT_CONFIG);
  const [workspaceContext, setWorkspaceContext] = useState("");

  const fetchData = useCallback(async () => {
    try {
      const [wsRes, cfgRes, ctxRes] = await Promise.all([
        fetch(`/api/workspaces/${id}`),
        fetch(`/api/workspaces/${id}/config`),
        fetch(`/api/workspaces/${id}/context`),
      ]);

      if (wsRes.ok) {
        const ws = await wsRes.json();
        setWorkspaceName(ws.name);
        setRepoPath(ws.repoPath);
      }

      if (cfgRes.ok) {
        setConfig(await cfgRes.json());
      }

      if (ctxRes.ok) {
        const ctx = await ctxRes.json();
        setWorkspaceContext(ctx.context || "");
      }
    } catch (error) {
      console.error("Failed to fetch:", error);
      toast.error("Failed to load configuration");
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  async function handleSave() {
    setSaving(true);
    try {
      const res = await fetch(`/api/workspaces/${id}/config`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(config),
      });
      if (res.ok) toast.success("Configuration saved");
      else throw new Error("Failed");
    } catch {
      toast.error("Failed to save");
    } finally {
      setSaving(false);
    }
  }

  async function handleSync() {
    setSyncing(true);
    try {
      const res = await fetch(`/api/workspaces/${id}/config/sync`, { method: "POST" });
      if (res.ok) {
        const data = await res.json();
        toast.success(`Synced to ${data.path}`);
      } else throw new Error("Failed");
    } catch {
      toast.error("Failed to sync");
    } finally {
      setSyncing(false);
    }
  }

  async function handleSaveContext() {
    setSavingContext(true);
    try {
      const res = await fetch(`/api/workspaces/${id}/context`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ context: workspaceContext }),
      });
      if (res.ok) toast.success("Workspace context saved");
      else throw new Error("Failed");
    } catch {
      toast.error("Failed to save context");
    } finally {
      setSavingContext(false);
    }
  }

  if (loading) {
    return (
      <div className="flex flex-col h-full">
        <Header title="Loading..." />
        <div className="flex-1 p-6 flex items-center justify-center text-muted-foreground">
          Loading configuration...
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <Header title={`Configure: ${workspaceName}`} description={repoPath} />

      <div className="flex-1 p-4 md:p-6 space-y-6 max-w-3xl overflow-auto">
        <Button variant="ghost" size="sm" onClick={() => router.push("/settings")}>
          <ArrowLeft className="h-4 w-4 mr-2" />
          Back to Settings
        </Button>

        <Card>
          <CardHeader>
            <CardTitle>Approval Rules</CardTitle>
            <CardDescription>Commands requiring human approval before execution</CardDescription>
          </CardHeader>
          <CardContent>
            <RuleListEditor
              label="Patterns"
              description="Partial matches supported (e.g., 'git push' matches 'git push origin main')"
              items={config.approvalRules}
              onChange={(rules) => setConfig({ ...config, approvalRules: rules })}
              placeholder="e.g., git push"
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Blocked Commands</CardTitle>
            <CardDescription>Commands that are always denied</CardDescription>
          </CardHeader>
          <CardContent>
            <RuleListEditor
              label="Patterns"
              items={config.blockedCommands}
              onChange={(cmds) => setConfig({ ...config, blockedCommands: cmds })}
              placeholder="e.g., rm -rf /"
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Custom Instructions</CardTitle>
            <CardDescription>Additional instructions for the agent</CardDescription>
          </CardHeader>
          <CardContent>
            <Textarea
              className="min-h-[120px]"
              value={config.customInstructions}
              onChange={(e) => setConfig({ ...config, customInstructions: e.target.value })}
              placeholder="Enter instructions..."
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Allowed Paths</CardTitle>
            <CardDescription>Restrict file operations to these paths (empty = all)</CardDescription>
          </CardHeader>
          <CardContent>
            <RuleListEditor
              label="Paths"
              items={config.allowedPaths}
              onChange={(paths) => setConfig({ ...config, allowedPaths: paths })}
              placeholder="e.g., src/"
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Workspace Context (WORKSPACE.md)</CardTitle>
            <CardDescription>
              Project knowledge passed to agents. Describe architecture, conventions, and important patterns.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <Textarea
              className="min-h-[300px] font-mono text-sm"
              value={workspaceContext}
              onChange={(e) => setWorkspaceContext(e.target.value)}
              placeholder="# Workspace Name

## Overview
Describe your project...

## Architecture
Key directories and patterns...

## Build & Test
npm run build, npm test, etc..."
            />
            <Button onClick={handleSaveContext} disabled={savingContext} size="sm">
              <Save className="h-4 w-4 mr-2" />
              {savingContext ? "Saving..." : "Save Context"}
            </Button>
          </CardContent>
        </Card>

        <div className="flex gap-3">
          <Button onClick={handleSave} disabled={saving}>
            <Save className="h-4 w-4 mr-2" />
            {saving ? "Saving..." : "Save Configuration"}
          </Button>
          <Button variant="outline" onClick={handleSync} disabled={syncing}>
            <RefreshCw className={`h-4 w-4 mr-2 ${syncing ? "animate-spin" : ""}`} />
            {syncing ? "Syncing..." : "Sync to Workspace"}
          </Button>
        </div>

        {/* Git Remote Settings */}
        <div className="pb-6">
          <h2 className="text-lg font-semibold mb-4">Git Remote</h2>
          <GitRemoteSettings workspaceId={parseInt(id)} />
        </div>
      </div>
    </div>
  );
}
