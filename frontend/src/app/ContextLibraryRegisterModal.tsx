import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../hooks/useInvoke";
import { baseName, pickFolder, terminalInputClass } from "./contextLibraryShared";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";

// ============================================================================
// New-project modal — the primary "add a project" flow.
//
// Default path: create a project at the managed CL location
// (`<data_dir>/library/projects/<name>/`, auto-seeded) via `cl_create_project`.
// The optional Working repo only BINDS the repo sessions run in — it is NOT
// indexed. The old "index an arbitrary on-disk folder as CL" behavior
// (`cl_path`) is demoted to a collapsed Advanced section for the rare doc-repo
// case. This fixes the trap where pointing the (formerly required) folder-path
// field at a code repo indexed the whole tree.
// ============================================================================

export function RegisterProjectModal({
  open,
  onClose,
  onRegistered,
}: {
  open: boolean;
  onClose: () => void;
  /** Fires with the registered project's name so the parent can focus it. */
  onRegistered: (name: string) => void;
}) {
  const [name, setName] = useState("");
  const [workingRepo, setWorkingRepo] = useState("");
  const [description, setDescription] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [clPath, setClPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trapRef = useFocusTrap<HTMLDivElement>(open);

  // Escape closes, mirroring ConfirmDialog. Guarded on `open` because this
  // modal stays mounted while hidden.
  useEscapeKey(onClose, open);

  if (!open) return null;

  // Suggest a name from whichever path the user filled (repo, then advanced
  // cl_path) so the common case is one field.
  const suggestedName =
    baseName(workingRepo.trim()) || baseName(clPath.trim()) || "";
  const effectiveName = name.trim() || suggestedName;
  const useClPath = advancedOpen && clPath.trim() !== "";

  const reset = () => {
    setName("");
    setWorkingRepo("");
    setDescription("");
    setAdvancedOpen(false);
    setClPath("");
    setError(null);
  };

  const browse = async (
    title: string,
    current: string,
    set: (v: string) => void,
  ) => {
    try {
      const picked = await pickFolder(title, current);
      if (picked) set(picked);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const submit = async () => {
    if (busy) return;
    if (!effectiveName) {
      setError("A project name is required.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (useClPath) {
        // Advanced: index an existing on-disk folder AS the CL content.
        await invoke("cl_register_project", {
          name: effectiveName,
          displayName: effectiveName,
          workingRepoPath: workingRepo.trim() || null,
          clPath: clPath.trim(),
          description: description.trim() || null,
        });
        await invoke("cl_rescan", { project: effectiveName });
      } else {
        // Default: managed CL location, repo bound but not indexed.
        await invoke("cl_create_project", {
          name: effectiveName,
          workingRepoPath: workingRepo.trim() || null,
          description: description.trim() || null,
        });
      }
      onRegistered(effectiveName);
      reset();
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const browseBtnClass =
    "shrink-0 rounded border border-outline-variant bg-transparent px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-label="New project"
      onClick={onClose}
    >
      <div
        ref={trapRef}
        tabIndex={-1}
        className="w-full max-w-md rounded border border-outline-variant bg-surface-container p-4 shadow-lg focus:outline-none"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-headline-md text-headline-md text-on-surface">
          New project
        </h2>
        <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
          Creates a Context Library project at the managed location. The working
          repo is bound for sessions — its files are <strong>not</strong>{" "}
          indexed.
        </p>

        <label className="mt-4 block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Name
          </span>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={suggestedName || "project-name"}
            autoFocus
            className={terminalInputClass}
          />
        </label>

        <label className="mt-3 block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Working repo <span className="opacity-60">(optional)</span>
          </span>
          <div className="flex items-center gap-2">
            <input
              type="text"
              value={workingRepo}
              onChange={(e) => setWorkingRepo(e.target.value)}
              placeholder="(where sessions run — leave blank for none)"
              className={terminalInputClass}
            />
            <button
              type="button"
              onClick={() =>
                browse("Choose working repo", workingRepo, setWorkingRepo)
              }
              className={browseBtnClass}
            >
              Browse…
            </button>
          </div>
        </label>

        <label className="mt-3 block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Description <span className="opacity-60">(optional)</span>
          </span>
          <input
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="one-line summary"
            className={terminalInputClass}
          />
        </label>

        <div className="mt-4 border-t border-outline-variant pt-3">
          <button
            type="button"
            onClick={() => setAdvancedOpen((v) => !v)}
            aria-expanded={advancedOpen}
            className="flex items-center gap-1 font-label-caps text-label-caps text-on-surface-variant transition-colors hover:text-on-surface"
          >
            <span
              aria-hidden
              className={`inline-block w-3 transition-transform ${advancedOpen ? "rotate-90" : ""}`}
            >
              ▸
            </span>
            Advanced
          </button>
          {advancedOpen && (
            <label className="mt-2 block">
              <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                Use an existing folder as CL content
              </span>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={clPath}
                  onChange={(e) => setClPath(e.target.value)}
                  placeholder="(advanced — for a docs folder, not a code repo)"
                  className={terminalInputClass}
                />
                <button
                  type="button"
                  onClick={() =>
                    browse("Choose CL content folder", clPath, setClPath)
                  }
                  className={browseBtnClass}
                >
                  Browse…
                </button>
              </div>
              <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
                Indexes that folder's docs as this project's Context Library.
                Leave empty to use the managed default (recommended).
              </p>
            </label>
          )}
        </div>

        {error && (
          <p className="mt-3 font-code-sm text-code-sm text-error">{error}</p>
        )}

        <div className="mt-4 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-outline-variant bg-transparent px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={busy || !effectiveName}
            onClick={submit}
            className="rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            {busy ? "Creating…" : useClPath ? "Index folder" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}
