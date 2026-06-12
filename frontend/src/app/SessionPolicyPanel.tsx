import { useEffect, useRef, useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import type { GatedKeyword, Policy } from "../lib/bindings";
import { PolicyForm } from "../components/PolicyForm";
import { GatedKeywordList } from "../components/GatedKeywordList";
import { CloseIcon, SaveIcon } from "./contextLibraryShared";
import { cn } from "../lib/cn";
import { useFocusTrap } from "../hooks/useFocusTrap";

/**
 * Right-side drawer for editing a session's canonical policy snapshot
 * (`.local/session-policies/<sid>.yaml`). A fixed inset-y right-side drawer
 * with Escape-to-close. Reads via `get_session_policy` — which
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

  // Trap focus in the drawer while open (Tab can't escape to the page behind).
  const trapRef = useFocusTrap<HTMLElement>(open);

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
      ref={trapRef}
      tabIndex={-1}
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
          way).
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

        <SessionToolGateSection sessionId={sessionId} />
      </div>
    </aside>
  );
}

// ----------------------------------------------------------------------------
// Per-session gated-Bash keywords. Seeded from the global Tool Gate at spawn;
// editable here for THIS session only (global stays the default for new
// sessions). The enforcement hook sources from this snapshot first, so edits
// are live on the next Bash call.
// ----------------------------------------------------------------------------

function SessionToolGateSection({ sessionId }: { sessionId: string }) {
  const { data: server = [], refetch, isLoading } = useTauriQuery<
    GatedKeyword[]
  >("get_session_tool_gate", { sessionId }, { enabled: !!sessionId });
  const save = useTauriMutation<
    void,
    { sessionId: string; keywords: GatedKeyword[] }
  >("set_session_tool_gate");

  const serverJson = JSON.stringify(server);
  const [draft, setDraft] = useState<GatedKeyword[]>(server);
  const lastServer = useRef(serverJson);
  useEffect(() => {
    if (lastServer.current !== serverJson) {
      lastServer.current = serverJson;
      setDraft(server);
    }
  }, [serverJson, server]);

  const dirty = JSON.stringify(draft) !== serverJson;
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!saved) return;
    const id = setTimeout(() => setSaved(false), 2000);
    return () => clearTimeout(id);
  }, [saved]);

  const onSave = async () => {
    await save.mutateAsync({
      sessionId,
      keywords: draft.filter((k) => k.keyword.trim() !== ""),
    });
    setSaved(true);
    refetch();
  };

  return (
    <section className="mt-8 border-t border-outline-variant/30 pt-5">
      <div className="mb-3 flex items-start justify-between gap-4">
        <div>
          <h3 className="font-headline-md text-headline-md text-on-surface">
            Gated commands — this session
          </h3>
          <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
            Seeded from the global Tool Gate at spawn; edits apply to this
            session only.{" "}
            <span className="text-primary">Gate</span> blocks a matching Bash
            command and asks you to Approve/Reject;{" "}
            <span className="text-emerald-300">Auto-allow</span> runs it with no
            prompt. Case-insensitive substring match.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {saved && !dirty && (
            <span className="rounded border border-emerald-500/40 bg-emerald-500/15 px-1.5 py-0.5 font-label-caps text-label-caps text-emerald-300">
              Saved ✓
            </span>
          )}
          {dirty && (
            <button
              type="button"
              onClick={onSave}
              disabled={save.isPending}
              className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
            >
              <SaveIcon />
              {save.isPending ? "Saving…" : "Save keywords"}
            </button>
          )}
        </div>
      </div>

      {isLoading ? (
        <div className="h-20 animate-pulse rounded-lg border border-outline-variant bg-surface-container" />
      ) : (
        <div className="rounded-lg border border-outline-variant bg-surface-container p-3">
          <GatedKeywordList
            value={draft}
            onChange={setDraft}
            placeholder="keyword (e.g. gh issue comment, rm -rf)"
            inputClassName="min-w-0 flex-1 border-0 border-b border-outline-variant bg-transparent rounded-none px-0 py-1 font-code-sm text-code-sm text-on-surface placeholder:text-on-surface-variant caret-primary focus:border-primary focus:outline-none"
            emptyState={
              <p className="py-1 font-code-sm text-code-sm text-on-surface-variant">
                No keywords — every Bash command runs ungated for this session.
              </p>
            }
            footer={(addRow) => (
              <div className="mt-3">
                <button
                  type="button"
                  onClick={addRow}
                  className="rounded border border-outline-variant bg-transparent px-2.5 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
                >
                  + Add keyword
                </button>
              </div>
            )}
          />
        </div>
      )}
      {save.error && (
        <p className="mt-3 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
          Save failed: {save.error.message}
        </p>
      )}
    </section>
  );
}
