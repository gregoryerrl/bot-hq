"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Eye, EyeOff, Loader2 } from "lucide-react";
import { toast } from "sonner";

interface Workspace {
  id: number;
  name: string;
}

interface RemoteFormProps {
  onSuccess: () => void;
  onCancel: () => void;
  editRemote?: {
    id: number;
    provider: string;
    name: string;
    url: string;
    owner?: string;
    repo?: string;
    workspaceId?: number;
  };
}

export function RemoteForm({ onSuccess, onCancel, editRemote }: RemoteFormProps) {
  const [provider, setProvider] = useState(editRemote?.provider || "github");
  const [name, setName] = useState(editRemote?.name || "");
  const [url, setUrl] = useState(editRemote?.url || "https://github.com");
  const [owner, setOwner] = useState(editRemote?.owner || "");
  const [repo, setRepo] = useState(editRemote?.repo || "");
  const [token, setToken] = useState("");
  const [showToken, setShowToken] = useState(false);
  const [workspaceId, setWorkspaceId] = useState<string>(editRemote?.workspaceId?.toString() || "global");
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    fetchWorkspaces();
  }, []);

  useEffect(() => {
    // Set default URL based on provider
    const defaultUrls: Record<string, string> = {
      github: "https://github.com",
      gitlab: "https://gitlab.com",
      bitbucket: "https://bitbucket.org",
      gitea: "",
      custom: "",
    };
    if (!editRemote) {
      setUrl(defaultUrls[provider] || "");
    }
  }, [provider, editRemote]);

  async function fetchWorkspaces() {
    try {
      const res = await fetch("/api/workspaces");
      if (res.ok) {
        const data = await res.json();
        setWorkspaces(data);
      }
    } catch (error) {
      console.error("Failed to fetch workspaces:", error);
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();

    if (!name.trim()) {
      toast.error("Please enter a name for this remote");
      return;
    }

    if (!url.trim()) {
      toast.error("Please enter a URL");
      return;
    }

    setSaving(true);
    try {
      const body = {
        provider,
        name: name.trim(),
        url: url.trim(),
        owner: owner.trim() || null,
        repo: repo.trim() || null,
        workspaceId: workspaceId === "global" ? null : parseInt(workspaceId),
        token: token.trim() || undefined,
      };

      const res = editRemote
        ? await fetch(`/api/git-remote/${editRemote.id}`, {
            method: "PATCH",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body),
          })
        : await fetch("/api/git-remote", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body),
          });

      if (!res.ok) {
        const error = await res.json();
        throw new Error(error.error || "Failed to save remote");
      }

      toast.success(editRemote ? "Remote updated" : "Remote added");
      onSuccess();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "Failed to save remote");
    } finally {
      setSaving(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="provider">Provider</Label>
        <Select value={provider} onValueChange={setProvider}>
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="github">GitHub</SelectItem>
            <SelectItem value="gitlab">GitLab</SelectItem>
            <SelectItem value="bitbucket">Bitbucket</SelectItem>
            <SelectItem value="gitea">Gitea</SelectItem>
            <SelectItem value="custom">Custom</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-2">
        <Label htmlFor="name">Name</Label>
        <Input
          id="name"
          placeholder="My GitHub Account"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          A display name for this remote
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="url">URL</Label>
        <Input
          id="url"
          placeholder="https://github.com"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          Base URL for the git provider
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="workspace">Scope</Label>
        <Select value={workspaceId} onValueChange={setWorkspaceId}>
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="global">Global (all workspaces)</SelectItem>
            {workspaces.map((ws) => (
              <SelectItem key={ws.id} value={ws.id.toString()}>
                {ws.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          Global remotes provide authentication for all workspaces
        </p>
      </div>

      {workspaceId !== "global" && (
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
      )}

      <div className="space-y-2">
        <Label htmlFor="token">Access Token</Label>
        <div className="relative">
          <Input
            id="token"
            type={showToken ? "text" : "password"}
            placeholder={editRemote ? "Leave empty to keep existing" : "ghp_xxxx..."}
            value={token}
            onChange={(e) => setToken(e.target.value)}
          />
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
            onClick={() => setShowToken(!showToken)}
          >
            {showToken ? (
              <EyeOff className="h-4 w-4 text-muted-foreground" />
            ) : (
              <Eye className="h-4 w-4 text-muted-foreground" />
            )}
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          {provider === "github" && (
            <>
              Get a token at{" "}
              <a
                href="https://github.com/settings/tokens"
                target="_blank"
                rel="noopener noreferrer"
                className="text-primary hover:underline"
              >
                github.com/settings/tokens
              </a>
            </>
          )}
          {provider === "gitlab" && (
            <>
              Get a token at{" "}
              <a
                href="https://gitlab.com/-/profile/personal_access_tokens"
                target="_blank"
                rel="noopener noreferrer"
                className="text-primary hover:underline"
              >
                GitLab Access Tokens
              </a>
            </>
          )}
          {provider === "bitbucket" && "Create an app password in Bitbucket settings"}
          {(provider === "gitea" || provider === "custom") && "Enter your access token"}
        </p>
      </div>

      <div className="flex gap-3 pt-4">
        <Button type="submit" disabled={saving}>
          {saving && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
          {editRemote ? "Update Remote" : "Add Remote"}
        </Button>
        <Button type="button" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
