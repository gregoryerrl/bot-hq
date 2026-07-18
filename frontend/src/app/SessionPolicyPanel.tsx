import { useEffect, useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { useServerDraft } from "../hooks/useServerDraft";
import type {
  GatedKeyword,
  Policy,
  SessionInfo,
  SessionProjectInfo,
} from "../lib/bindings";
import { PolicyForm } from "../components/PolicyForm";
import { GatedKeywordList } from "../components/GatedKeywordList";
import { CloseIcon, SaveIcon } from "./contextLibraryShared";
import { cn } from "../lib/cn";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";

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
  session,
  sessionId,
  open,
  onClose,
}: {
  session: SessionInfo | null;
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
  const { draft, setDraft, dirty } = useServerDraft<Policy>(server ?? {});
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!saved) return;
    const id = setTimeout(() => setSaved(false), 2000);
    return () => clearTimeout(id);
  }, [saved]);

  // Trap focus in the drawer while open (Tab can't escape to the page behind).
  const trapRef = useFocusTrap<HTMLElement>(open);

  // Escape-to-close, scoped to open so it doesn't fight other handlers.
  useEscapeKey(onClose, open);

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
            <span className="rounded border border-warning/40 bg-warning/15 px-1.5 py-0.5 font-label-caps text-label-caps text-warning">
              Unsaved
            </span>
          )}
          {saved && !dirty && (
            <span className="rounded border border-success/40 bg-success/15 px-1.5 py-0.5 font-label-caps text-label-caps text-success">
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

      <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-5 py-5">
        <p className="mb-4 rounded border border-outline-variant/40 bg-surface-container/60 px-3 py-2 font-code-sm text-code-sm text-on-surface-variant">
          This session's policy. Push / force-push enforcement
          is <span className="text-on-surface">live</span> — the git hooks and
          MCP tools re-read this snapshot on every call. The agents' own
          system-prompt copy was fixed at spawn, so restart the session if you
          want them to <em>see</em> the new rules (enforcement applies either
          way).
        </p>
        <PolicyOriginBadge sessionId={sessionId} open={open} />
        {session?.brian_model_at_spawn && (
          <div className="mb-4 rounded border border-outline-variant/40 bg-surface-container/60 px-3 py-2">
            <p className="mb-1 font-label-caps text-label-caps text-on-surface-variant">
              Models (captured at spawn)
            </p>
            <p className="font-code-sm text-code-sm text-on-surface-variant">
              Brian:{" "}
              <span className="text-on-surface">
                {session.brian_model_at_spawn}
              </span>
              <span className="mx-2 text-outline-variant">·</span>
              Rain:{" "}
              <span className="text-on-surface">
                {session.rain_enabled
                  ? (session.rain_model_at_spawn ?? "—")
                  : "off"}
              </span>
            </p>
          </div>
        )}
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

/**
 * Surfaces HOW this session's project (and therefore its policy) was resolved —
 * registered repo vs path-basename inference vs no project. Closes the
 * 2026-06-11 "why am I getting the full inherited policy?" gap: an
 * unregistered repo silently inheriting general policy is now visible.
 */
function PolicyOriginBadge({
  sessionId,
  open,
}: {
  sessionId: string;
  open: boolean;
}) {
  const { data: info } = useTauriQuery<SessionProjectInfo>(
    "get_session_project_info",
    { sessionId },
    { enabled: open && !!sessionId },
  );
  if (!info) return null;
  const { project, provenance } = info;
  const badgeCls = {
    registered: "border-success/40 bg-success/15 text-success",
    inferred: "border-warning/40 bg-warning/15 text-warning",
    none: "border-outline-variant bg-surface-container text-on-surface-variant",
  }[provenance];
  const badgeLabel =
    provenance === "registered"
      ? `${project} · registered`
      : provenance === "inferred"
        ? `${project} · inferred from path`
        : "no project";
  const explanation =
    provenance === "registered"
      ? `Resolved from the registered project "${project}" — its policy.yaml (if any) narrows the rules below.`
      : provenance === "inferred"
        ? `This repo isn't a registered project; "${project}" is just its folder name, so general policy applies until you register it.`
        : "Repo-less session — general policy applies by inheritance.";
  return (
    <div className="mb-4 rounded border border-outline-variant/40 bg-surface-container/60 px-3 py-2">
      <div className="mb-1 flex items-center gap-2">
        <span className="font-label-caps text-label-caps text-on-surface-variant">
          Policy origin
        </span>
        <span
          className={cn(
            "rounded border px-1.5 py-0.5 font-label-caps text-label-caps",
            badgeCls,
          )}
        >
          {badgeLabel}
        </span>
      </div>
      <p className="font-code-sm text-code-sm text-on-surface-variant">
        {explanation}
      </p>
    </div>
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

  const { draft, setDraft, dirty } = useServerDraft<GatedKeyword[]>(server);
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
            <span className="text-success">Auto-allow</span> runs it with no
            prompt. Case-insensitive substring match.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {saved && !dirty && (
            <span className="rounded border border-success/40 bg-success/15 px-1.5 py-0.5 font-label-caps text-label-caps text-success">
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
