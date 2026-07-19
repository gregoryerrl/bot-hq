import { memo, useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";
import { PhasePillRow, type Phase } from "./PhasePill";
import { ChoicePrompt, type ChoicePromptChoice } from "./ChoicePrompt";
import { Markdown } from "./Markdown";
import { ErrorBanner } from "./ErrorBanner";
import { cn } from "../lib/cn";
import { formatRelative } from "../lib/time";
import { groupDiffByFile, type DiffLine } from "../lib/diffGroups";
import type {
  ResolveResult,
  SessionDocumentView,
  SessionTrayView,
} from "../lib/bindings";

// Relative age for stale-gate context, with an "earlier" fallback for a
// null/unparseable timestamp. Delegates to the shared, zone-safe formatRelative
// (parseUtcMs) — a bare Date.parse misreads a zone-less timestamp as local time
// (the "stale 8h" hallucination).
function relAgo(iso: string | null): string {
  return (iso ? formatRelative(iso) : "") || "earlier";
}

interface DocumentPaneProps {
  sessionId: string;
  /**
   * The session's current IPAV phase. The pane's visible tab FOLLOWS this so
   * docs don't appear to "disappear" (#3): the tab used to hardcode-default to
   * "investigate" and never sync, so once work moved to Plan/Apply/Verify the
   * Investigate tab showed empty ("No investigate documents yet") even though
   * docs existed under the active phase. The user can still click other tabs
   * to peek; the view re-follows on the next phase change.
   */
  sessionPhase?: Phase | null;
}

interface ComputeApplyDiffResult {
  lines: DiffLine[];
  note: string | null;
}

export const DocumentPane = memo(function DocumentPane({
  sessionId,
  sessionPhase,
}: DocumentPaneProps) {
  const [activePhase, setActivePhase] = useState<Phase>(
    sessionPhase ?? "investigate",
  );
  // The Tray tab sits before I/P/A/V and is phase-independent: it's the
  // session's pending inbox — questions / approvals / gated commands awaiting
  // the user's input, answered inline. A phase transition updates the
  // underlying phase but does NOT yank the user off the Tray.
  const [showTray, setShowTray] = useState(false);

  // Follow the session's phase whenever it changes (the fix for #3). Firing
  // only on sessionPhase change means a manual tab click still sticks until
  // the next phase transition, so the user can freely peek at other phases.
  useEffect(() => {
    if (sessionPhase) setActivePhase(sessionPhase);
  }, [sessionPhase]);

  const { data: docs = [], error: docsError } = useTauriQuery<
    SessionDocumentView[]
  >("session_doc_search", { sessionId, phase: activePhase });

  // Apply tab gets a live git diff above the phase=apply session docs.
  // Refetches on session change; Apply tab visibility doesn't matter for the
  // query — TanStack Query caches it for instant switch-back.
  const { data: applyDiff, error: diffError } =
    useTauriQuery<ComputeApplyDiffResult>(
      "compute_apply_diff",
      { sessionId },
      { enabled: !!sessionId && activePhase === "apply" },
    );

  // Stable ref across unrelated re-renders (Tray toggle, TL;DR state) so the
  // memoized <Markdown> children below aren't needlessly reconciled (O7).
  const activeDocs = useMemo(
    () => docs.filter((d) => d.phase === activePhase),
    [docs, activePhase],
  );

  // Pending count for the Tray pill badge — shows even on the I/P/A/V tabs so
  // accumulated input is visible without opening the Tray. Shares the
  // list_session_tray cache with TrayList (same query key); GlobalEventSync
  // invalidates it event-driven, so the badge updates live.
  const { data: trayEntries = [] } = useTauriQuery<SessionTrayView[]>(
    "list_session_tray",
    { sessionId },
  );
  const pendingTrayCount = useMemo(
    () => trayEntries.filter((e) => e.status === "pending").length,
    [trayEntries],
  );

  // One-shot "TL;DR" summary of a doc, rendered in a dispose-on-close dialog.
  // The backend `summarize_session_doc` runs a headless model in the
  // background; we just hold the latest request's status here.
  const [summary, setSummary] = useState<{
    slug: string;
    status: "loading" | "done" | "error";
    text: string;
  } | null>(null);
  const runSummary = async (slug: string) => {
    setSummary({ slug, status: "loading", text: "" });
    try {
      const text = await invoke<string>("summarize_session_doc", {
        sessionId,
        slug,
      });
      setSummary({ slug, status: "done", text });
    } catch (e) {
      setSummary({ slug, status: "error", text: errorMessage(e) });
    }
  };

  return (
    <div className="flex h-full min-w-0 flex-col border-l border-outline-variant bg-surface-container-lowest/50">
      <div className="flex items-center gap-1 border-b border-outline-variant px-3 py-2">
        <TrayPill
          selected={showTray}
          onSelect={() => setShowTray(true)}
          count={pendingTrayCount}
        />
        <span className="mx-1 h-4 w-px bg-outline-variant" aria-hidden />
        <PhasePillRow
          selected={showTray ? null : activePhase}
          onSelect={(p) => {
            setShowTray(false);
            setActivePhase(p);
          }}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-3">
        {showTray ? (
          <TrayList sessionId={sessionId} />
        ) : (
          <>
            {activePhase === "apply" &&
              (diffError ? (
                <p className="mb-3 text-sm text-on-error-container">
                  Couldn't compute the diff: {diffError.message}
                </p>
              ) : (
                <ApplyDiffBlock diff={applyDiff ?? null} />
              ))}
            {activeDocs.length === 0 ? (
              docsError ? (
                <p className="text-sm text-on-error-container">
                  Couldn't load {activePhase} documents: {docsError.message}
                </p>
              ) : activePhase === "apply" && applyDiff?.lines.length ? null : (
                <p className="text-sm text-on-surface-variant">
                  No {activePhase} documents yet.
                </p>
              )
            ) : (
              activeDocs.map((doc) => (
                <article key={doc.id} className="mb-6">
                  <header className="mb-2 flex items-center justify-between gap-2">
                    <h4 className="min-w-0 truncate text-sm font-semibold text-on-surface">
                      {doc.slug}
                    </h4>
                    <div className="flex shrink-0 items-center gap-2">
                      <button
                        type="button"
                        onClick={() => runSummary(doc.slug)}
                        disabled={
                          summary?.slug === doc.slug &&
                          summary.status === "loading"
                        }
                        title="Summarize this document (TL;DR) with a background model"
                        className="rounded border border-outline-variant bg-transparent px-2 py-0.5 text-[0.6rem] font-semibold uppercase tracking-wide text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
                      >
                        {summary?.slug === doc.slug &&
                        summary.status === "loading"
                          ? "…"
                          : "TL;DR"}
                      </button>
                      <span className="text-[0.65rem] text-on-surface-variant">
                        {doc.updated_at}
                      </span>
                    </div>
                  </header>
                  <Markdown>{doc.body}</Markdown>
                </article>
              ))
            )}
          </>
        )}
      </div>
      <SummaryDialog
        open={summary !== null}
        slug={summary?.slug ?? ""}
        status={summary?.status ?? "loading"}
        text={summary?.text ?? ""}
        onClose={() => setSummary(null)}
      />
    </div>
  );
});

// One-shot TL;DR modal: a scrim + focus-trapped panel mirroring ConfirmDialog,
// but display-only (single Close). Backdrop click and Escape both dismiss; the
// caller clears its state on close, disposing the summary.
function SummaryDialog({
  open,
  slug,
  status,
  text,
  onClose,
}: {
  open: boolean;
  slug: string;
  status: "loading" | "done" | "error";
  text: string;
  onClose: () => void;
}) {
  const trapRef = useFocusTrap<HTMLDivElement>(open);
  useEscapeKey(onClose, open);

  if (!open) return null;

  return (
    <>
      <div
        className="fixed inset-0 z-40 bg-black/60"
        onClick={onClose}
        aria-hidden
      />
      <div
        ref={trapRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={`Summary of ${slug}`}
        className={cn(
          "fixed left-1/2 top-1/2 z-50 max-h-[80vh] w-[min(560px,90vw)] -translate-x-1/2 -translate-y-1/2 overflow-y-auto overflow-x-hidden",
          "rounded-lg border border-outline-variant bg-surface-container p-5 shadow-2xl focus:outline-none",
        )}
      >
        <div className="mb-3 flex items-center justify-between gap-2">
          <h2 className="min-w-0 truncate text-base font-semibold text-on-surface">
            TL;DR — {slug}
          </h2>
          <span className="shrink-0 text-[0.6rem] uppercase tracking-wide text-on-surface-variant">
            summary
          </span>
        </div>
        <div className="mb-5 min-h-[3rem] text-sm text-on-surface-variant">
          {status === "loading" ? (
            <p className="animate-pulse">Summarizing…</p>
          ) : status === "error" ? (
            <p className="text-on-error-container">
              Couldn't summarize: {text}
            </p>
          ) : (
            <Markdown>{text}</Markdown>
          )}
        </div>
        <div className="flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-outline-variant bg-surface-container-high px-3 py-1.5 text-sm text-on-surface transition-colors hover:bg-surface-container-highest"
          >
            Close
          </button>
        </div>
      </div>
    </>
  );
}

// GitHub dark-mode palette: green adds, red removes, blue hunk headers,
// yellow file headers, muted context. Mirrors the Rust diff classifier
// `parse_diff_lines` in `tauri_cmd/docs.rs`.
const diffLineClass: Record<string, string> = {
  add: "bg-success/10 text-success",
  remove: "bg-error/10 text-error",
  hunk: "bg-tertiary/10 text-tertiary",
  file: "text-warning",
  context: "text-on-surface-variant",
};

function ApplyDiffBlock({ diff }: { diff: ComputeApplyDiffResult | null }) {
  // Per-file collapse state, keyed by group index. Reset whenever the diff
  // changes (new session / fresh apply) so each visit starts fully expanded.
  const [closed, setClosed] = useState<Set<number>>(() => new Set());
  useEffect(() => {
    setClosed(new Set());
  }, [diff]);

  // Grouping is O(diff lines) — derive it once per diff, not on every render
  // (collapse toggles and parent re-renders would otherwise re-scan a
  // potentially thousands-of-lines diff).
  const groups = useMemo(
    () => (diff ? groupDiffByFile(diff.lines) : []),
    [diff],
  );

  if (!diff) {
    return (
      <p className="mb-6 text-xs text-on-surface-variant">Loading diff…</p>
    );
  }
  if (diff.lines.length === 0) {
    return (
      <p className="mb-6 text-xs text-on-surface-variant">
        {diff.note ?? "(no changes)"}
      </p>
    );
  }
  // "All closed" drives the toggle's label/action. A fresh `closed` set means
  // everything is open → button reads "Collapse all".
  const allClosed = groups.length > 0 && closed.size === groups.length;
  const toggleAll = () =>
    setClosed(allClosed ? new Set() : new Set(groups.map((_, i) => i)));
  return (
    <section className="mb-6 rounded border border-outline-variant bg-surface-container-lowest">
      <header className="flex items-center justify-between gap-2 border-b border-outline-variant px-3 py-1.5 text-[0.65rem] uppercase tracking-wide text-on-surface-variant">
        <span>git diff{diff.note ? ` — ${diff.note}` : ""}</span>
        {groups.length > 1 && (
          <button
            type="button"
            onClick={toggleAll}
            className="shrink-0 rounded border border-outline-variant px-1.5 py-0.5 font-semibold uppercase tracking-wide text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
            title={allClosed ? "Expand all files" : "Collapse all files"}
          >
            {allClosed ? "Expand all" : "Collapse all"}
          </button>
        )}
      </header>
      <div className="divide-y divide-outline-variant">
        {groups.map((g, gi) => (
          <details
            key={gi}
            open={!closed.has(gi)}
            // Sync native toggle (user clicking a file header) back into state so
            // the collapse-all label stays accurate when files are toggled by hand.
            onToggle={(e) => {
              const isOpen = e.currentTarget.open;
              setClosed((prev) => {
                const next = new Set(prev);
                if (isOpen) next.delete(gi);
                else next.add(gi);
                return next;
              });
            }}
            className="group"
          >
            <summary className="flex cursor-pointer list-none items-center justify-between gap-2 px-3 py-1.5 text-[0.7rem] hover:bg-surface-container-low">
              <span className="flex min-w-0 items-center gap-1.5">
                <span
                  aria-hidden
                  className="text-on-surface-variant transition-transform group-open:rotate-90"
                >
                  ▶
                </span>
                <span className="truncate font-mono text-warning">
                  {g.file}
                </span>
              </span>
              <span className="shrink-0 font-mono text-[0.65rem]">
                {g.adds > 0 && (
                  <span className="text-success">+{g.adds}</span>
                )}
                {g.adds > 0 && g.removes > 0 && " "}
                {g.removes > 0 && (
                  <span className="text-error">−{g.removes}</span>
                )}
              </span>
            </summary>
            <pre className="whitespace-pre-wrap break-words px-3 py-2 text-[0.7rem] leading-relaxed font-mono">
              {g.lines.map((line, i) => (
          <div
            key={i}
            className={cn(
              "whitespace-pre-wrap",
              diffLineClass[line.kind] ?? "text-on-surface-variant",
            )}
          >
            {line.text || " "}
          </div>
              ))}
            </pre>
          </details>
        ))}
      </div>
    </section>
  );
}

// ---- Tray tab ----------------------------------------------------------

function TrayPill({
  selected,
  onSelect,
  count,
}: {
  selected: boolean;
  onSelect: () => void;
  count: number;
}) {
  const hasPending = count > 0;
  return (
    <button
      onClick={onSelect}
      className={cn(
        "inline-flex items-center gap-1 rounded px-2 py-1 text-xs font-semibold uppercase border-t-2",
        selected
          ? "border-on-surface/70 bg-surface-container-high/80 text-on-surface"
          : hasPending
            ? "border-primary/70 bg-primary/10 text-primary animate-pulse"
            : "border-transparent bg-transparent text-on-surface-variant hover:text-on-surface",
      )}
      title="Session tray — pending questions, approvals & gated commands awaiting your input"
    >
      Tray
      {hasPending && (
        <span className="inline-flex min-w-[1.1rem] items-center justify-center rounded-full bg-primary px-1 text-[0.6rem] font-semibold text-on-primary">
          {count}
        </span>
      )}
    </button>
  );
}

// Actionable pending inbox: pending tray items for this session, answered
// inline. Resolved history is intentionally NOT shown (it's noise — the tray
// is an inbox, not an audit log). Reads the durable table so items that
// accumulated while the user was AFK (and survived a restart) still appear.
function TrayList({ sessionId }: { sessionId: string }) {
  const queryClient = useQueryClient();
  // Refreshed event-driven by GlobalEventSync (invalidates queries on the
  // session:* events) — newly-parked pending appear and answered items drop
  // without a poll or a manual tab-switch.
  const { data: entries = [] } = useTauriQuery<SessionTrayView[]>(
    "list_session_tray",
    { sessionId },
  );
  // Track which (choiceId, option) is mid-resolve so the clicked option shows
  // "…" and the row disables until resolve_choice settles + the tray refetches
  // (the answered item then drops out of the pending filter).
  const [resolving, setResolving] = useState<Map<string, string>>(new Map());
  const [resolveError, setResolveError] = useState<string | null>(null);
  // When approving a STALE gated command (its requesting agent has moved on),
  // the backend returns needs_stale_confirm instead of running it — we hold the
  // pick here and surface a confirmation before re-resolving with confirmStale.
  const [staleConfirm, setStaleConfirm] = useState<{
    choiceId: string;
    picked: string;
    command: string;
    askedAt: string | null;
  } | null>(null);

  const onResolve = (choiceId: string, picked: string, confirmStale = false) => {
    setResolving((m) => new Map(m).set(choiceId, picked));
    setResolveError(null);
    invoke<ResolveResult>("resolve_choice", { choiceId, picked, confirmStale })
      .then((res) => {
        // Stale gate: don't run the (possibly now-invalid) command on a blind
        // approve — park the pick and ask the user to confirm first.
        if (res.kind === "needs_stale_confirm") {
          setStaleConfirm({
            choiceId,
            picked,
            command: res.command,
            askedAt: res.asked_at,
          });
        }
      })
      // Surface the failure — answering is the core HITL action, and a silent
      // console.error left the item stuck pending with no signal to the user.
      .catch((e) => setResolveError(errorMessage(e)))
      .finally(() => {
        setResolving((m) => {
          const next = new Map(m);
          next.delete(choiceId);
          return next;
        });
        void queryClient.invalidateQueries({
          queryKey: ["list_session_tray", { sessionId }],
        });
      });
  };

  const pending = entries.filter((e) => e.status === "pending");
  if (pending.length === 0) {
    return (
      <p className="text-sm text-on-surface-variant">
        No pending input — you're all caught up.
      </p>
    );
  }
  return (
    <>
      {resolveError && (
        <ErrorBanner
          label="Couldn't submit your answer:"
          message={resolveError}
          onDismiss={() => setResolveError(null)}
          className="mb-3"
        />
      )}
      {staleConfirm && (
        <div
          role="alertdialog"
          className="mb-3 rounded border border-error/50 bg-error-container/30 px-3 py-2 text-xs text-on-error-container"
        >
          <p className="font-semibold">
            ⚠ This command's requesting agent has moved on
          </p>
          <p className="mt-1">
            Requested {relAgo(staleConfirm.askedAt)} by an agent that has since
            timed out. The repo state may have changed — running it now could be
            invalid or destructive.
          </p>
          <pre className="mt-1 whitespace-pre-wrap break-words rounded bg-surface-container-high px-2 py-1 font-mono text-on-surface-variant">
            {staleConfirm.command}
          </pre>
          <div className="mt-2 flex gap-2">
            <button
              type="button"
              className="rounded bg-error px-2 py-1 font-semibold text-on-error hover:opacity-90"
              onClick={() => {
                const sc = staleConfirm;
                setStaleConfirm(null);
                onResolve(sc.choiceId, sc.picked, true);
              }}
            >
              Run anyway
            </button>
            <button
              type="button"
              className="rounded border border-outline/40 px-2 py-1 hover:bg-surface-container-high"
              onClick={() => setStaleConfirm(null)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
      <ul className="space-y-3">
        {pending.map((e) => (
        <li key={e.id}>
          <TrayChoice
            entry={e}
            sessionId={sessionId}
            pendingOption={resolving.get(e.choice_id)}
            onResolve={onResolve}
          />
        </li>
        ))}
      </ul>
    </>
  );
}

// One pending tray item, answered via the shared ChoicePrompt (preset options
// + mandatory "Other"). Shows the kind/agent and, for an action_gate approval,
// the gated command above the prompt for context.
function TrayChoice({
  entry,
  sessionId,
  pendingOption,
  onResolve,
}: {
  entry: SessionTrayView;
  sessionId: string;
  pendingOption: string | undefined;
  onResolve: (choiceId: string, picked: string) => void;
}) {
  const choice: ChoicePromptChoice = {
    choice_id: entry.choice_id,
    session_id: sessionId,
    agent: entry.agent,
    question: entry.prompt,
    options: entry.options,
  };
  return (
    <div>
      <div className="mb-1 flex items-center gap-2">
        <span className="rounded bg-surface-container-high px-1.5 py-0.5 text-[0.6rem] uppercase tracking-wide text-on-surface-variant">
          {entry.kind}
        </span>
        <span className="text-[0.7rem] text-on-surface-variant">
          {entry.agent}
        </span>
      </div>
      {entry.command_text && (
        <pre className="mb-1 whitespace-pre-wrap break-words rounded bg-surface-container-high px-2 py-1 text-[0.7rem] font-mono text-on-surface-variant">
          {entry.command_text}
        </pre>
      )}
      {entry.stale && (
        <div className="mb-1 rounded border border-error/40 bg-error-container/20 px-2 py-1 text-[0.7rem] text-on-error-container">
          ⚠ Stale — requested {relAgo(entry.asked_at)}; the requesting agent has
          moved on. Review the command before approving (repo state may have
          changed).
        </div>
      )}
      <ChoicePrompt
        choice={choice}
        pendingOption={pendingOption}
        onResolve={onResolve}
      />
    </div>
  );
}
