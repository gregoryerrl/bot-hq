import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../hooks/useInvoke";
import type { ClFolderView } from "../lib/bindings";
import { FolderIcon, type OpenTab, terminalInputClass } from "./contextLibraryShared";

// ============================================================================
// FolderView — editor-pane content for a folder tab. Edits the folder's CL
// description + tags (cl_folders) via cl_set_folder_description. The current
// value is read from the already-fetched folder list (same pattern as the file
// editor's DescriptionEditor reading from `entries`), so no extra fetch.
// `folderPath === ""` is the project-root folder.
// ============================================================================

export function FolderView({
  tab,
  folders,
  onSaved,
}: {
  tab: Extract<OpenTab, { kind: "folder" }>;
  folders: ClFolderView[];
  onSaved: () => void;
}) {
  const current =
    folders.find(
      (f) => f.project_id === tab.project && f.folder_path === tab.folderPath,
    ) ?? null;
  const isRoot = tab.folderPath === "";
  const initialDesc = current?.description ?? "";
  const initialTags = current?.tags ?? "";

  // Re-seed when the active folder changes (also keyed at the EditorArea level,
  // so this is belt-and-suspenders — mirrors the file DescriptionEditor).
  const seedKey = `${tab.project}/${tab.folderPath}`;
  const [seed, setSeed] = useState(seedKey);
  const [desc, setDesc] = useState(initialDesc);
  const [tagsStr, setTagsStr] = useState(initialTags);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (seed !== seedKey) {
    setSeed(seedKey);
    setDesc(initialDesc);
    setTagsStr(initialTags);
    setError(null);
  }

  const dirty = desc !== initialDesc || tagsStr !== initialTags;

  const handleSave = async () => {
    if (!dirty || saving) return;
    setSaving(true);
    setError(null);
    try {
      await invoke("cl_set_folder_description", {
        project: tab.project,
        folderPath: tab.folderPath,
        description: desc,
        tags: tagsStr.trim() ? tagsStr.trim() : null,
      });
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background">
      <header className="flex flex-shrink-0 items-center gap-2 border-b border-outline-variant bg-surface-container-low px-4 py-2">
        <FolderIcon className="shrink-0 text-on-surface-variant/60" />
        <div className="min-w-0">
          <p className="truncate font-code-sm text-code-sm text-on-surface">
            {isRoot ? tab.project : tab.folderPath}
          </p>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {isRoot ? "project root" : tab.project}
          </p>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
        <p className="mb-2 font-label-caps text-label-caps text-on-surface-variant/70">
          Folder metadata · cl_folders
        </p>
        <label className="block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Description
          </span>
          <textarea
            rows={3}
            value={desc}
            onChange={(e) => setDesc(e.target.value)}
            placeholder="What this folder holds — shown in cl_folder_search."
            className="w-full resize-y rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm text-on-surface placeholder:text-on-surface-variant focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
          />
        </label>
        <label className="mt-3 block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Tags
          </span>
          <input
            type="text"
            value={tagsStr}
            onChange={(e) => setTagsStr(e.target.value)}
            placeholder="(optional, comma-separated)"
            className={terminalInputClass}
          />
        </label>
        {error && (
          <p className="mt-2 font-code-sm text-code-sm text-error">
            Save failed: {error}{" "}
            <button onClick={() => setError(null)} className="underline">
              dismiss
            </button>
          </p>
        )}
        <div className="mt-3 flex items-center justify-end gap-2">
          <button
            type="button"
            disabled={!dirty || saving}
            onClick={() => {
              setDesc(initialDesc);
              setTagsStr(initialTags);
              setError(null);
            }}
            className="rounded border border-outline-variant bg-transparent px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
          >
            Reset
          </button>
          <button
            type="button"
            disabled={!dirty || saving}
            onClick={handleSave}
            className="rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            {saving ? "Saving…" : "Save folder"}
          </button>
        </div>
      </div>
    </div>
  );
}
