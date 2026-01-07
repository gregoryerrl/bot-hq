"use client";

import { useState, useEffect, use } from "react";
import { useRouter } from "next/navigation";
import { Header } from "@/components/layout/header";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { RuleListEditor } from "@/components/settings/rule-list-editor";
import { ArrowLeft, Save, RefreshCw } from "lucide-react";
import { AgentConfig, DEFAULT_AGENT_CONFIG } from "@/lib/agents/config-types";
import { toast } from "sonner";

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
  const [workspaceName, setWorkspaceName] = useState("");
  const [repoPath, setRepoPath] = useState("");
  const [config, setConfig] = useState<AgentConfig>(DEFAULT_AGENT_CONFIG);

  useEffect(() => {
    fetchData();
  }, [id]);

  async function fetchData() {
    try {
      const [wsRes, cfgRes] = await Promise.all([
        fetch(`/api/workspaces/${id}`),
        fetch(`/api/workspaces/${id}/config`),
      ]);

      if (wsRes.ok) {
        const ws = await wsRes.json();
        setWorkspaceName(ws.name);
        setRepoPath(ws.repoPath);
      }

      if (cfgRes.ok) {
        setConfig(await cfgRes.json());
      }
    } catch (error) {
      console.error("Failed to fetch:", error);
      toast.error("Failed to load configuration");
    } finally {
      setLoading(false);
    }
  }

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

        <div className="flex gap-3 pb-6">
          <Button onClick={handleSave} disabled={saving}>
            <Save className="h-4 w-4 mr-2" />
            {saving ? "Saving..." : "Save Configuration"}
          </Button>
          <Button variant="outline" onClick={handleSync} disabled={syncing}>
            <RefreshCw className={`h-4 w-4 mr-2 ${syncing ? "animate-spin" : ""}`} />
            {syncing ? "Syncing..." : "Sync to Workspace"}
          </Button>
        </div>
      </div>
    </div>
  );
}
