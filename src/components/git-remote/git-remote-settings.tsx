"use client";

import { useState, useEffect } from "react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { GitBranch, Save, Loader2, CheckCircle2, AlertCircle, Trash2 } from "lucide-react";
import { toast } from "sonner";

interface GitRemote {
  id: number;
  provider: string;
  name: string;
  url: string;
  owner: string | null;
  repo: string | null;
  hasCredentials: boolean;
}

interface GitRemoteSettingsProps {
  workspaceId: number;
}

export function GitRemoteSettings({ workspaceId }: GitRemoteSettingsProps) {
  const [remote, setRemote] = useState<GitRemote | null>(null);
  const [globalRemotes, setGlobalRemotes] = useState<GitRemote[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  // Form state
  const [selectedGlobalRemote, setSelectedGlobalRemote] = useState<string>("");
  const [owner, setOwner] = useState("");
  const [repo, setRepo] = useState("");

  useEffect(() => {
    fetchRemotes();
  }, [workspaceId]);

  async function fetchRemotes() {
    try {
      const res = await fetch("/api/git-remote");
      if (res.ok) {
        const remotes = await res.json();

        // Find workspace-specific remote
        const workspaceRemote = remotes.find(
          (r: any) => r.workspaceId === workspaceId
        );

        // Get global remotes for dropdown
        const globals = remotes.filter((r: any) => !r.workspaceId);

        setRemote(workspaceRemote || null);
        setGlobalRemotes(globals);

        if (workspaceRemote) {
          setOwner(workspaceRemote.owner || "");
          setRepo(workspaceRemote.repo || "");
        }
      }
    } catch (error) {
      console.error("Failed to fetch remotes:", error);
    } finally {
      setLoading(false);
    }
  }

  async function handleSave() {
    if (!owner.trim() || !repo.trim()) {
      toast.error("Please enter both owner and repository name");
      return;
    }

    setSaving(true);
    try {
      if (remote) {
        // Update existing remote
        const res = await fetch(`/api/git-remote/${remote.id}`, {
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ owner: owner.trim(), repo: repo.trim() }),
        });

        if (!res.ok) throw new Error("Failed to update");
        toast.success("Git remote updated");
      } else if (selectedGlobalRemote) {
        // Create workspace remote linked to global
        const globalRemote = globalRemotes.find(r => r.id.toString() === selectedGlobalRemote);
        if (!globalRemote) return;

        const res = await fetch("/api/git-remote", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            provider: globalRemote.provider,
            name: `${globalRemote.name} - Workspace`,
            url: globalRemote.url,
            workspaceId,
            owner: owner.trim(),
            repo: repo.trim(),
          }),
        });

        if (!res.ok) throw new Error("Failed to create");
        toast.success("Git remote configured");
      } else {
        toast.error("Please select a global remote first");
        return;
      }

      fetchRemotes();
    } catch (error) {
      toast.error("Failed to save git remote settings");
    } finally {
      setSaving(false);
    }
  }

  async function handleRemove() {
    if (!remote) return;
    if (!confirm("Remove git remote configuration for this workspace?")) return;

    try {
      const res = await fetch(`/api/git-remote/${remote.id}`, { method: "DELETE" });
      if (res.ok) {
        toast.success("Git remote removed");
        setRemote(null);
        setOwner("");
        setRepo("");
      }
    } catch (error) {
      toast.error("Failed to remove git remote");
    }
  }

  if (loading) {
    return (
      <Card>
        <CardContent className="py-6">
          <div className="flex items-center justify-center text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin mr-2" />
            Loading...
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <GitBranch className="h-5 w-5" />
          Git Remote
        </CardTitle>
        <CardDescription>
          Connect this workspace to a remote repository
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {remote ? (
          // Existing remote configuration
          <>
            <div className="flex items-center gap-2 mb-4">
              <Badge variant="outline">{remote.provider}</Badge>
              {remote.hasCredentials ? (
                <Badge variant="outline" className="text-green-600 border-green-600">
                  <CheckCircle2 className="h-3 w-3 mr-1" />
                  Connected
                </Badge>
              ) : (
                <Badge variant="outline" className="text-yellow-600 border-yellow-600">
                  <AlertCircle className="h-3 w-3 mr-1" />
                  No Token
                </Badge>
              )}
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-2">
                <Label htmlFor="owner">Owner/Organization</Label>
                <Input
                  id="owner"
                  placeholder="username or org"
                  value={owner}
                  onChange={(e) => setOwner(e.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="repo">Repository</Label>
                <Input
                  id="repo"
                  placeholder="repo-name"
                  value={repo}
                  onChange={(e) => setRepo(e.target.value)}
                />
              </div>
            </div>

            <div className="flex gap-2">
              <Button onClick={handleSave} disabled={saving} size="sm">
                {saving && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
                <Save className="h-4 w-4 mr-2" />
                Save
              </Button>
              <Button onClick={handleRemove} variant="outline" size="sm">
                <Trash2 className="h-4 w-4 mr-2" />
                Remove
              </Button>
            </div>
          </>
        ) : (
          // No remote configured
          <>
            {globalRemotes.length === 0 ? (
              <div className="text-center py-4 text-muted-foreground">
                <p>No global git remotes configured.</p>
                <p className="text-sm mt-1">
                  Add a remote on the{" "}
                  <a href="/git-remote" className="text-primary hover:underline">
                    Git Remote
                  </a>{" "}
                  page first.
                </p>
              </div>
            ) : (
              <>
                <div className="space-y-2">
                  <Label>Use Remote</Label>
                  <Select value={selectedGlobalRemote} onValueChange={setSelectedGlobalRemote}>
                    <SelectTrigger>
                      <SelectValue placeholder="Select a remote..." />
                    </SelectTrigger>
                    <SelectContent>
                      {globalRemotes.map((r) => (
                        <SelectItem key={r.id} value={r.id.toString()}>
                          {r.name} ({r.provider})
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {selectedGlobalRemote && (
                  <>
                    <div className="grid grid-cols-2 gap-3">
                      <div className="space-y-2">
                        <Label htmlFor="owner">Owner/Organization</Label>
                        <Input
                          id="owner"
                          placeholder="username or org"
                          value={owner}
                          onChange={(e) => setOwner(e.target.value)}
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="repo">Repository</Label>
                        <Input
                          id="repo"
                          placeholder="repo-name"
                          value={repo}
                          onChange={(e) => setRepo(e.target.value)}
                        />
                      </div>
                    </div>

                    <Button onClick={handleSave} disabled={saving} size="sm">
                      {saving && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
                      Configure Remote
                    </Button>
                  </>
                )}
              </>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}
