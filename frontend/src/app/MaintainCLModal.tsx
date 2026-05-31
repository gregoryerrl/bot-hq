import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import { maintainClPrompt } from "../lib/maintainClPrompt";
import type { ProjectView, SessionInfo } from "../lib/bindings";

interface MaintainCLModalProps {
  open: boolean;
  onClose: () => void;
}

/**
 * "Maintain CL" dispatcher. Pick a project; this creates a session, spawns the
 * Brian + Rain duo, and seeds it with the hardcoded CL-maintenance prompt (all
 * via the `dispatch_session` command), then navigates into it. The duo starts
 * maintaining that project's Context Library immediately — no follow-up needed.
 */
export function MaintainCLModal({ open, onClose }: MaintainCLModalProps) {
  const navigate = useNavigate();
  const { data: projects = [] } = useTauriQuery<ProjectView[]>(
    "list_projects",
    {},
    { refetchInterval: 60_000, enabled: open },
  );
  const [selected, setSelected] = useState("");
  const [dispatching, setDispatching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selectRef = useRef<HTMLSelectElement | null>(null);

  // Focus the project select on open + Escape-to-dismiss. Mirrors the New
  // Session dialog in Dashboard.
  useEffect(() => {
    if (!open) return;
    setError(null);
    selectRef.current?.focus();
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

  const handleDispatch = async () => {
    if (!selected || dispatching) return;
    setDispatching(true);
    setError(null);
    try {
      const id = `s-${crypto.randomUUID().slice(0, 8)}`;
      const proj = projects.find((p) => p.name === selected);
      await invoke<SessionInfo>("dispatch_session", {
        id,
        title: `Maintain CL: ${selected}`,
        project: selected,
        repoPath: proj?.working_repo_path ?? null,
        prompt: maintainClPrompt(selected),
      });
      onClose();
      navigate(`/sessions/${id}`);
    } catch (e) {
      setError(errorMessage(e));
      setDispatching(false);
    }
  };

  return (
    <>
      <div
        className="fixed inset-0 z-40 bg-black/60"
        onClick={onClose}
        aria-hidden
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Dispatch CL maintenance"
        className={cn(
          "fixed left-1/2 top-1/2 z-50 w-[min(480px,90vw)] -translate-x-1/2 -translate-y-1/2",
          "rounded-lg border border-outline-variant bg-surface-container p-5 shadow-2xl",
        )}
      >
        <div className="mb-4 flex items-center justify-between">
          <h2 className="font-headline-md text-headline-md text-on-surface">
            Maintain Context Library
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="text-on-surface-variant hover:text-on-surface"
          >
            ×
          </button>
        </div>
        <p className="mb-3 font-code-sm text-code-sm text-on-surface-variant">
          Dispatches a Brian + Rain session that maintains the chosen project's
          CL — auditing the where-things-live map, sharpening descriptions, and
          pruning stale notes.
        </p>
        <label className="block">
          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
            Project
          </span>
          <select
            ref={selectRef}
            value={selected}
            onChange={(e) => setSelected(e.target.value)}
            className={cn(
              "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
              "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
            )}
          >
            <option value="">(choose a project)</option>
            {projects.map((p) => (
              <option key={p.name} value={p.name}>
                {p.display_name || p.name}
              </option>
            ))}
          </select>
        </label>
        {error && (
          <p className="mt-3 rounded border border-error/40 bg-error-container/30 px-3 py-2 text-xs text-on-error-container">
            {error}
          </p>
        )}
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleDispatch}
            disabled={!selected || dispatching}
          >
            {dispatching ? "Dispatching…" : "Start maintenance"}
          </Button>
        </div>
      </div>
    </>
  );
}
