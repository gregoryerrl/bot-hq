"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { FolderOpen, Save, X } from "lucide-react";

export function ScopeDirectory() {
  const [scopePath, setScopePath] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  useEffect(() => {
    fetchScopePath();
  }, []);

  async function fetchScopePath() {
    try {
      const res = await fetch("/api/settings?key=scope_path");
      if (res.ok) {
        const data = await res.json();
        setScopePath(data.value);
        setEditValue(data.value);
      }
    } catch (err) {
      console.error("Failed to fetch scope path:", err);
    }
  }

  async function handleSave() {
    if (!editValue.trim()) {
      setError("Path cannot be empty");
      return;
    }

    setLoading(true);
    setError(null);
    setSuccess(false);

    try {
      const res = await fetch("/api/settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          key: "scope_path",
          value: editValue,
        }),
      });

      if (res.ok) {
        setScopePath(editValue);
        setEditing(false);
        setSuccess(true);
        setTimeout(() => setSuccess(false), 3000);
      } else {
        const data = await res.json();
        setError(data.error || "Failed to save scope path");
      }
    } catch (err) {
      console.error("Failed to save scope path:", err);
      setError("Failed to save scope path");
    } finally {
      setLoading(false);
    }
  }

  function handleCancel() {
    setEditValue(scopePath);
    setEditing(false);
    setError(null);
  }

  function handleSelectDirectory() {
    // Create a hidden file input and trigger it
    const input = document.createElement("input");
    input.type = "file";
    // @ts-ignore - webkitdirectory is not in the TypeScript definitions
    input.webkitdirectory = true;
    input.multiple = false;

    input.onchange = (e: Event) => {
      const target = e.target as HTMLInputElement;
      if (target.files && target.files.length > 0) {
        // Get the directory path from the first file's path
        const firstFile = target.files[0];
        // @ts-ignore - webkitRelativePath is not in the TypeScript definitions
        const fullPath = firstFile.webkitRelativePath;
        if (fullPath) {
          // Extract just the directory path (remove the filename)
          const dirPath = fullPath.split("/")[0];
          // This won't give us the absolute path in a web context
          // We'll need to handle this differently
          setError("Directory picker is not fully supported in web browsers. Please type the path manually.");
        }
      }
    };

    input.click();
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Scope Directory</CardTitle>
        <CardDescription>
          Configure the root directory where all workspaces and projects are located.
          Agents will only work within this scope.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {!editing ? (
          <div className="flex items-center gap-2">
            <div className="flex-1 p-2 bg-muted rounded-md font-mono text-sm">
              {scopePath || "Not configured"}
            </div>
            <Button onClick={() => setEditing(true)} variant="outline">
              Edit
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <Input
                value={editValue}
                onChange={(e) => {
                  setEditValue(e.target.value);
                  setError(null);
                }}
                placeholder="/Users/username/Projects"
                className="flex-1 font-mono text-sm"
              />
              <Button
                onClick={handleSelectDirectory}
                variant="outline"
                size="icon"
                title="Select Directory"
              >
                <FolderOpen className="h-4 w-4" />
              </Button>
            </div>

            {error && (
              <div className="text-sm text-destructive">
                {error}
              </div>
            )}

            <div className="flex gap-2">
              <Button
                onClick={handleSave}
                disabled={loading}
                size="sm"
              >
                <Save className="h-4 w-4 mr-1" />
                {loading ? "Saving..." : "Save"}
              </Button>
              <Button
                onClick={handleCancel}
                variant="outline"
                size="sm"
              >
                <X className="h-4 w-4 mr-1" />
                Cancel
              </Button>
            </div>
          </div>
        )}

        {success && (
          <div className="text-sm text-green-600 dark:text-green-400">
            Scope path updated successfully
          </div>
        )}

        <div className="text-sm text-muted-foreground space-y-1">
          <p><strong>Fallback order:</strong></p>
          <ol className="list-decimal list-inside ml-2 space-y-1">
            <li>Database setting (editable here)</li>
            <li>SCOPE_PATH environment variable</li>
            <li>~/Projects (default)</li>
          </ol>
        </div>
      </CardContent>
    </Card>
  );
}
