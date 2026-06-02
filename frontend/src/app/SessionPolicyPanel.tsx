import { useEffect, useRef, useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import type { Policy } from "../lib/bindings";
import { PolicyForm } from "../components/PolicyForm";
import { CloseIcon, SaveIcon } from "./contextLibraryShared";
import { cn } from "../lib/cn";

/**
 * Right-side drawer for editing a session's canonical policy snapshot
 * (`.local/session-policies/<sid>.yaml`). Mirrors the EmmaOverlay drawer idiom
 * (fixed inset-y, Escape-to-close). Reads via `get_session_policy` — which
 * returns the snapshot verbatim when seeded, else the resolved general+project
 * blueprint, so the form shows real values even before the agents finish
 * spawning. Writes via `set_session_policy`, which preserves the frozen
 * tool_gate keywords (those are managed globally in Settings → Tool Gate).
 */
export function SessionPolicyPanel({
  sessionId,
  open,
  onClose,
}: {
  sessionId: string;
  open: boolean;
  onClose: () => void;
}) {
  const { data: server, refetch, isLoading } = useTauriQuery<Policy>(
    "get_session_policy",
    { sessionId },
    { enabled: open && !!sessionId },
  );
  const save = useTauriMutation<void, { sessionId: string; policy: Policy }>(
    "set_session_policy",
  );

  // Re-hydrate the draft whenever the server snapshot changes (initial load +
  // post-save refetch), matching the Settings draft/dirty idiom.
  const serverJson = JSON.stringify(server ?? {});
  const [draft, setDraft] = useState<Policy>(server ?? {});
  const lastServer = useRef(serverJson);
  useEffect(() => {
    if (lastServer.current !== serverJson) {
      lastServer.current = serverJson;
      setDraft(server ?? {});
    }
  }, [serverJson, server]);

  const dirty = JSON.stringify(draft) !== serverJson;
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!saved) return;
    const id = setTimeout(() => setSaved(false), 2000);
    return () => clearTimeout(id);
  }, [saved]);

  // Escape-to-close, scoped to open so it doesn't fight other handlers.
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

  const onSave = async () => {
    await save.mutateAsync({ sessionId, policy: draft });
    setSaved(true);
    refetch();
  };

  return (
    <aside
      role="dialog"
      aria-label="Session settings"
      className={cn(
        "fixed inset-y-0 right-0 z-30 flex w-full flex-col",
        "border-l border-outline-variant bg-background shadow-2xl md:w-1/2",
      )}
    >
      <header className="flex h-12 flex-shrink-0 items-center justify-between border-b border-outline-variant bg-surface-container px-4">
        <div className="flex items-center gap-2">
          <h2 className="font-headline-md text-headline-md text-on-surface">
            Session Settings
          </h2>
          {dirty && (
            <span className="rounded border border-amber-500/40 bg-amber-500/15 px-1.5 py-0.5 font-label-caps text-label-caps text-amber-300">
              Unsaved
            </span>
          )}
          {saved && !dirty && (
            <span className="rounded border border-emerald-500/40 bg-emerald-500/15 px-1.5 py-0.5 font-label-caps text-label-caps text-emerald-300">
              Saved ✓
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={!dirty || save.isPending}
            onClick={onSave}
            className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            {save.isPending ? "Saving…" : "Save"}
          </button>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close session settings"
            className="rounded p-1 text-on-surface-variant transition-colors hover:text-on-surface"
          >
            <CloseIcon />
          </button>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-auto px-5 py-5">
        <p className="mb-4 rounded border border-outline-variant/40 bg-surface-container/60 px-3 py-2 font-code-sm text-code-sm text-on-surface-variant">
          This session's policy. Push / force-push / forbidden-word enforcement
          is <span className="text-on-surface">live</span> — the git hooks and
          MCP tools re-read this snapshot on every call. The agents' own
          system-prompt copy was fixed at spawn, so restart the session if you
          want them to <em>see</em> the new rules (enforcement applies either
          way). Gated-Bash keywords are managed globally in{" "}
          <span className="text-on-surface">Settings → Tool Gate</span> and are
          preserved here.
        </p>
        {isLoading ? (
          <div className="h-40 animate-pulse rounded-lg border border-outline-variant bg-surface-container" />
        ) : (
          <PolicyForm value={draft} onChange={setDraft} disabled={save.isPending} />
        )}
        {save.error && (
          <p className="mt-4 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
            Save failed: {save.error.message}
          </p>
        )}
      </div>
    </aside>
  );
}
