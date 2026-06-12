import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../hooks/useInvoke";
import { baseName, terminalInputClass } from "./contextLibraryShared";
import { useFocusTrap } from "../hooks/useFocusTrap";

// ============================================================================
// RegisterProjectModal — promote an arbitrary on-disk folder to a Context
// Library project. The backend (cl_register_project) validates the path is a
// real directory; we then cl_rescan it so its files populate the tree. Uses a
// text input for the path (the pre-migration flow did too); a native folder
// picker would need the Tauri dialog plugin and can be added later.
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
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [workingRepo, setWorkingRepo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trapRef = useFocusTrap<HTMLDivElement>(open);

  // Escape closes, mirroring ConfirmDialog. Guarded on `open` because this
  // modal stays mounted while hidden.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const trimmedPath = path.trim();
  const suggestedName = trimmedPath ? baseName(trimmedPath) : "";
  const effectiveName = name.trim() || suggestedName;

  const reset = () => {
    setPath("");
    setName("");
    setWorkingRepo("");
    setError(null);
  };

  const submit = async () => {
    if (busy) return;
    if (!trimmedPath || !effectiveName) {
      setError("Folder path and name are required.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await invoke("cl_register_project", {
        name: effectiveName,
        displayName: effectiveName,
        workingRepoPath: workingRepo.trim() || null,
        clPath: trimmedPath,
        description: null,
      });
      // Index the new project's files so it appears in the tree immediately.
      await invoke("cl_rescan", { project: effectiveName });
      onRegistered(effectiveName);
      reset();
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-label="Register project"
      onClick={onClose}
    >
      <div
        ref={trapRef}
        tabIndex={-1}
        className="w-full max-w-md rounded border border-outline-variant bg-surface-container p-4 shadow-lg focus:outline-none"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-headline-md text-headline-md text-on-surface">
          Register project
        </h2>
        <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
          Index an on-disk folder as a Context Library project.
        </p>

        <label className="mt-4 block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Folder path
          </span>
          <input
            type="text"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="/Users/you/Projects/my-project"
            autoFocus
            className={terminalInputClass}
          />
        </label>
        <label className="mt-3 block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Name
          </span>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={suggestedName || "project-name"}
            className={terminalInputClass}
          />
        </label>
        <label className="mt-3 block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Working repo path (optional)
          </span>
          <input
            type="text"
            value={workingRepo}
            onChange={(e) => setWorkingRepo(e.target.value)}
            placeholder="(defaults to none)"
            className={terminalInputClass}
          />
        </label>

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
            disabled={busy || !trimmedPath}
            onClick={submit}
            className="rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            {busy ? "Registering…" : "Register"}
          </button>
        </div>
      </div>
    </div>
  );
}
