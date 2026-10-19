import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../hooks/useInvoke";
import { baseName, pickFolder } from "./contextLibraryShared";
import { ChevronRightIcon } from "../components/icons";
import { Button } from "../components/ui/Button";
import { Dialog } from "../components/ui/Dialog";
import { fieldClass } from "../components/ui/field";

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

  // The one Dialog (S2f): Escape, the scrim and the close button all call
  // `onClose`. Its scrim is the frame's sibling — this one used to be the
  // frame's parent, so a drag-select released over it closed the modal.
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="New project"
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            disabled={busy || !effectiveName}
            onClick={submit}
          >
            {busy ? "Creating…" : useClPath ? "Index folder" : "Create"}
          </Button>
        </>
      }
    >
      <p className="font-body-md text-body-md text-muted">
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
          className={fieldClass}
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
            className={fieldClass}
          />
          <Button
            variant="secondary"
            size="sm"
            onClick={() =>
              browse("Choose working repo", workingRepo, setWorkingRepo)
            }
            className="shrink-0"
          >
            Browse…
          </Button>
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
          className={fieldClass}
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
            <ChevronRightIcon className="size-3" />
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
                className={fieldClass}
              />
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  browse("Choose CL content folder", clPath, setClPath)
                }
                className="shrink-0"
              >
                Browse…
              </Button>
            </div>
            <p className="mt-1 font-body-md text-body-md text-muted">
              Indexes that folder's docs as this project's Context Library.
              Leave empty to use the managed default (recommended).
            </p>
          </label>
        )}
      </div>

      {error && (
        <p className="mt-3 font-code-sm text-code-sm text-error">{error}</p>
      )}
    </Dialog>
  );
}
