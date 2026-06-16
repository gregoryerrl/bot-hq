import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../hooks/useInvoke";
import type { ClFolderView, ProjectView } from "../lib/bindings";
import {
  FolderIcon,
  type OpenTab,
  pickFolder,
  terminalInputClass,
} from "./contextLibraryShared";
import { ConfirmDialog } from "../components/ConfirmDialog";

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
  project,
  onSaved,
  onProjectChanged,
  onProjectGone,
}: {
  tab: Extract<OpenTab, { kind: "folder" }>;
  folders: ClFolderView[];
  project: ProjectView | null;
  onSaved: () => void;
  onProjectChanged: () => void;
  /** Called after a project is deleted (no replacement) or renamed (replacement
   *  = new name) so the parent can close the stale tab + retarget. Optional so
   *  the folder-view still renders standalone (tests). */
  onProjectGone?: (name: string, replacement?: string) => void;
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

        {isRoot && project && (
          <ProjectSection
            project={project}
            onProjectChanged={onProjectChanged}
            onProjectGone={onProjectGone}
          />
        )}
      </div>
    </div>
  );
}

// ============================================================================
// ProjectSection — shown only on a project-root folder-view. Configures the
// project's working-repo, renames it, and removes it (Unbind = soft, keeps
// content; Delete = hard, purges rows + optional managed files). Creating a NEW
// project happens via the sidebar's New-project modal.
// ============================================================================

function ProjectSection({
  project,
  onProjectChanged,
  onProjectGone,
}: {
  project: ProjectView;
  onProjectChanged: () => void;
  onProjectGone?: (name: string, replacement?: string) => void;
}) {
  const [wr, setWr] = useState(project.working_repo_path ?? "");
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [newName, setNewName] = useState(project.name);
  const [showUnbindConfirm, setShowUnbindConfirm] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-seed when switching to a different project root.
  const [seededName, setSeededName] = useState(project.name);
  if (seededName !== project.name) {
    setSeededName(project.name);
    setWr(project.working_repo_path ?? "");
    setNewName(project.name);
    setRenaming(false);
    setError(null);
  }

  // Managed = default convention location (no custom cl_path). Only then is it
  // safe to offer "delete files on disk" — a custom cl_path is the user's own
  // folder/repo and must never be removed.
  const managed = !(project.cl_path && project.cl_path.trim());
  const wrDirty = wr.trim() !== (project.working_repo_path ?? "").trim();
  const renameDirty = newName.trim() !== "" && newName.trim() !== project.name;

  const saveWorkingRepo = async () => {
    if (!wrDirty || busy) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("cl_register_project", {
        name: project.name,
        displayName: project.display_name,
        workingRepoPath: wr.trim() || null,
        clPath: null,
        description: null,
      });
      onProjectChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const browseRepo = async () => {
    try {
      const picked = await pickFolder("Choose working repo", wr);
      if (picked) setWr(picked);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const doRename = async () => {
    if (!renameDirty || busy) return;
    setBusy(true);
    setError(null);
    try {
      const target = newName.trim();
      await invoke("cl_rename_project", { name: project.name, newName: target });
      setRenaming(false);
      // Old root tab is now stale — close it and open the renamed root.
      if (onProjectGone) onProjectGone(project.name, target);
      else onProjectChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const unbind = async () => {
    if (busy) return;
    setShowUnbindConfirm(false);
    setBusy(true);
    setError(null);
    try {
      await invoke("cl_unregister_project", { name: project.name });
      onProjectChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const doDelete = async () => {
    if (busy) return;
    setShowDeleteConfirm(false);
    setBusy(true);
    setError(null);
    try {
      await invoke("cl_delete_project", {
        name: project.name,
        deleteClDir: managed && deleteFiles,
      });
      if (onProjectGone) onProjectGone(project.name);
      else onProjectChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const browseBtnClass =
    "shrink-0 rounded border border-outline-variant bg-transparent px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface";

  return (
    <div className="mt-6 border-t border-outline-variant pt-4">
      <p className="mb-2 font-label-caps text-label-caps text-on-surface-variant/70">
        Project · registration
      </p>
      <p className="mb-3 font-code-sm text-code-sm text-on-surface-variant">
        CL path:{" "}
        <span className="text-on-surface">
          {project.cl_path || "(default convention)"}
        </span>
      </p>

      {/* Name + rename */}
      <div className="mb-3">
        <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
          Name
        </span>
        {renaming ? (
          <div className="flex items-center gap-2">
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              autoFocus
              className={terminalInputClass}
            />
            <button
              type="button"
              disabled={!renameDirty || busy}
              onClick={doRename}
              className="shrink-0 rounded border border-primary bg-primary px-2 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
            >
              Save
            </button>
            <button
              type="button"
              onClick={() => {
                setRenaming(false);
                setNewName(project.name);
              }}
              className={browseBtnClass}
            >
              Cancel
            </button>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            <span className="truncate font-code-sm text-code-sm text-on-surface">
              {project.name}
            </span>
            <button
              type="button"
              disabled={busy}
              onClick={() => setRenaming(true)}
              className={browseBtnClass}
            >
              Rename
            </button>
          </div>
        )}
      </div>

      <label className="block">
        <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
          Working repo path
        </span>
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={wr}
            onChange={(e) => setWr(e.target.value)}
            placeholder="(none)"
            className={terminalInputClass}
          />
          <button type="button" onClick={browseRepo} className={browseBtnClass}>
            Browse…
          </button>
        </div>
      </label>
      {error && (
        <p className="mt-2 font-code-sm text-code-sm text-error">{error}</p>
      )}
      <div className="mt-3 flex items-center justify-end">
        <button
          type="button"
          disabled={!wrDirty || busy}
          onClick={saveWorkingRepo}
          className="rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
        >
          Save working repo
        </button>
      </div>

      {/* Removal actions */}
      <div className="mt-4 flex items-center justify-between gap-2 border-t border-outline-variant pt-3">
        <button
          type="button"
          disabled={busy}
          onClick={() => setShowUnbindConfirm(true)}
          title="Clears the working-repo + custom CL path. Keeps files & descriptions."
          className="rounded border border-outline-variant bg-transparent px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
        >
          Unbind working repo
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setDeleteFiles(false);
            setShowDeleteConfirm(true);
          }}
          title="Permanently removes the project + its CL index from bot-hq."
          className="rounded border border-error/50 bg-transparent px-3 py-1 font-code-sm text-code-sm text-error transition-colors hover:bg-error/10 disabled:opacity-50"
        >
          Delete project
        </button>
      </div>

      <ConfirmDialog
        open={showUnbindConfirm}
        title="Unbind working repo?"
        message={
          <>
            Unbind{" "}
            <strong className="text-on-surface">{project.name}</strong>? Files and
            descriptions are kept — this only clears its working-repo + custom CL
            path.
          </>
        }
        confirmLabel="Unbind"
        confirmVariant="danger"
        onConfirm={unbind}
        onCancel={() => setShowUnbindConfirm(false)}
      />
      <ConfirmDialog
        open={showDeleteConfirm}
        title="Delete project?"
        message={
          <>
            Permanently delete{" "}
            <strong className="text-on-surface">{project.name}</strong> and its
            Context Library index? This cannot be undone.
            {managed ? (
              <label className="mt-3 flex items-center gap-2 font-code-sm text-code-sm text-on-surface-variant">
                <input
                  type="checkbox"
                  checked={deleteFiles}
                  onChange={(e) => setDeleteFiles(e.target.checked)}
                  className="size-4 accent-primary"
                />
                Also delete the CL files on disk
              </label>
            ) : (
              <span className="mt-3 block font-code-sm text-code-sm text-on-surface-variant">
                Files at the custom CL path are left untouched.
              </span>
            )}
          </>
        }
        confirmLabel="Delete"
        confirmVariant="danger"
        onConfirm={doDelete}
        onCancel={() => setShowDeleteConfirm(false)}
      />
    </div>
  );
}
