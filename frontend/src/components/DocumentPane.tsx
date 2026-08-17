import { memo, useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";
import { PhasePillRow, type Phase } from "./PhasePill";
import { isTrayItem } from "./HaltBanner";
import { useTrayStaging, stagedFor } from "../stores/trayStaging";
import { ChoicePrompt, type ChoicePromptChoice } from "./ChoicePrompt";
import { Markdown } from "./Markdown";
import { ErrorBanner } from "./ErrorBanner";
import { ConfirmDialog } from "./ConfirmDialog";
import { FileViewerDialog, fileArgInCommand } from "./FileViewerDialog";
import { TrashIcon } from "./icons";
import { Button } from "./ui/Button";
import { cn } from "../lib/cn";
import { formatRelative } from "../lib/time";
import { groupDiffByFile, type DiffLine } from "../lib/diffGroups";
import { authorLabel, useParticipantLabels } from "../lib/participants";
import type {
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
  // rc3 D35: the tray holds QUESTIONS. A halt is a declared state (the
  // banner); an approval is the gate. Counting either here is how a badge said
  // "one item on tray" over a tray with nothing in it.
  const pendingTrayCount = useMemo(
    () =>
      trayEntries.filter((e) => e.status === "pending" && isTrayItem(e)).length,
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
      {/* min-w-0: without it a flex item's `min-width:auto` refuses to shrink
          below its content, so a long gated command pushes the pane wider than
          its parent and overflow-x-hidden clips it out of reach. */}
      <div className="min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-3">
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
  // rc3 D10: a tray card is asked BY a participant, and `entry.agent` is that
  // participant's stored slug — an internal key, and on legacy rows literally
  // "brian" / "rain". The roster turns it into `ROLE · Model`; the tray was
  // named in the sweep list and was still printing the raw field.
  const { labels } = useParticipantLabels(sessionId);
  // Refreshed event-driven by GlobalEventSync (invalidates queries on the
  // session:* events) — newly-parked pending appear and answered items drop
  // without a poll or a manual tab-switch.
  const { data: entries = [] } = useTauriQuery<SessionTrayView[]>(
    "list_session_tray",
    { sessionId },
  );
  const [resolveError, setResolveError] = useState<string | null>(null);
  // Discard = bin a card WITHOUT answering it (nothing is sent to the agent).
  // Confirmed first: a mis-click on a real approval gate is destructive — a
  // discarded push gate denies the push.
  const [discardTarget, setDiscardTarget] = useState<SessionTrayView | null>(
    null,
  );
  // Full-screen preview: either a file the gated command references, or the
  // command itself when it's too long to read inside the card.
  const [viewFile, setViewFile] = useState<{
    sessionId: string;
    path: string;
  } | null>(null);
  const [viewInline, setViewInline] = useState<{
    title: string;
    text: string;
  } | null>(null);

  const onDiscardConfirmed = (choiceId: string) => {
    setDiscardTarget(null);
    setResolveError(null);
    invoke<boolean>("discard_choice", { choiceId })
      .catch((e) => setResolveError(errorMessage(e)))
      .finally(() => {
        void queryClient.invalidateQueries({
          queryKey: ["list_session_tray", { sessionId }],
        });
      });
  };

  // The per-card resolve path is GONE (rc3 D35). Questions stage and travel
  // with the composer's Send; approvals are answered in the gate, which owns
  // the stale-confirm flow too. Discard is the one direct action left here.

  // **rc3 D35: the tray holds QUESTIONS and nothing else.** A halt is a
  // declared state — the banner above the input box; an approval is the gate,
  // which replaces the input box and halts the session. Rendering either here
  // gave one fact two surfaces, and the user's report was a badge counting a
  // halt as "one item on tray" over a tray with nothing in it.
  const pending = entries.filter((e) => e.status === "pending" && isTrayItem(e));
  // rc3 D34/D35: a click STAGES the pick — always. There is no send on a
  // question; the composer's Send delivers every staged answer with the typed
  // message as ONE response. The user: "I thought I was clear on this that
  // answers will be sent in one batch. REMOVE THE SEND BUTTON ON QUESTIONS."
  const staged = useTrayStaging((s) => stagedFor(s.staged, sessionId));
  const stage = useTrayStaging((s) => s.stage);
  const unstage = useTrayStaging((s) => s.unstage);
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
      <ConfirmDialog
        open={discardTarget !== null}
        title="Discard this card?"
        message={
          <>
            <p className="mb-2">
              It's removed from your tray and <strong>nothing is sent to the
              agent</strong> — no answer, no rejection. It won't be asked again
              unless the agent re-raises it.
            </p>
            {discardTarget?.command_text && (
              <p className="rounded border border-error/40 bg-error-container/20 px-2 py-1">
                This is an approval gate. Discarding it counts as{" "}
                <strong>not approved</strong>, so the command will not run.
              </p>
            )}
          </>
        }
        confirmLabel="Discard"
        confirmVariant="danger"
        onConfirm={() =>
          discardTarget && onDiscardConfirmed(discardTarget.choice_id)
        }
        onCancel={() => setDiscardTarget(null)}
      />
      <FileViewerDialog
        target={viewFile}
        inlineTitle={viewInline?.title}
        inlineText={viewInline?.text}
        onClose={() => {
          setViewFile(null);
          setViewInline(null);
        }}
      />
      <ul className="space-y-3">
        {pending.map((e) => (
        <li key={e.id}>
          <TrayChoice
            entry={e}
            sessionId={sessionId}
            askedByLabel={authorLabel(e.agent, labels)}
            stagedOption={staged[e.choice_id]}
            onStage={(choiceId, picked) => stage(sessionId, choiceId, picked)}
            onUnstage={() => unstage(sessionId, e.choice_id)}
            onDiscard={() => setDiscardTarget(e)}
            onViewFile={(path) => setViewFile({ sessionId, path })}
            onExpand={(title, text) => setViewInline({ title, text })}
          />
        </li>
        ))}
      </ul>
    </>
  );
}

// One pending tray item, answered via the shared ChoicePrompt (preset options
// + mandatory "Other"). Shows the kind + who asked and, for an action_gate
// approval, the gated command above the prompt for context.
function TrayChoice({
  entry,
  sessionId,
  askedByLabel,
  stagedOption,
  onStage,
  onUnstage,
  onDiscard,
  onViewFile,
  onExpand,
}: {
  entry: SessionTrayView;
  sessionId: string;
  /** Who asked, as `ROLE · Model` — resolved by the list, which holds the
   *  roster. Never `entry.agent` itself (rc3 D10). */
  askedByLabel: string;
  /** rc3 D34: the pick staged for the next Send, if any. Staged ≠ answered —
   *  the row is still pending and the agent has seen nothing yet. */
  stagedOption?: string | undefined;
  /** Stage a pick (option click or Other text). There is no per-card send
   *  (rc3 D35) — the composer's Send delivers everything as one response. */
  onStage: (choiceId: string, picked: string) => void;
  onUnstage?: () => void;
  onDiscard: () => void;
  onViewFile: (path: string) => void;
  onExpand: (title: string, text: string) => void;
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
          {askedByLabel}
        </span>
        <button
          type="button"
          onClick={onDiscard}
          aria-label="Discard this card without answering"
          title="Discard — removes it from your tray; nothing is sent to the agent"
          className="ml-auto rounded p-1 text-on-surface-variant transition-colors hover:bg-error-container/30 hover:text-error"
        >
          <TrashIcon />
        </button>
      </div>
      {entry.command_text && (
        <div className="mb-1">
          {/* `break-words` (overflow-wrap: break-word) only breaks a token that
              can't fit a line ALONE; a long unbroken run of a command still
              overflowed and got clipped by the pane's overflow-x-hidden.
              `anywhere` breaks at any point, and max-h keeps a giant issue body
              from swallowing the card — Expand shows it in full. */}
          <pre className="max-h-48 overflow-y-auto overflow-x-hidden whitespace-pre-wrap [overflow-wrap:anywhere] rounded bg-surface-container-high px-2 py-1 text-[0.7rem] font-mono text-on-surface-variant">
            {entry.command_text}
          </pre>
          {/* A gate that names a file shows only the PATH — the body being
              approved is invisible without this. */}
          <div className="mt-1 flex flex-wrap gap-1.5">
            {(() => {
              const f = fileArgInCommand(entry.command_text);
              return f ? (
                <Button size="sm" variant="ghost" onClick={() => onViewFile(f)}>
                  View {f.split("/").pop()}
                </Button>
              ) : null;
            })()}
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onExpand("Gated command", entry.command_text ?? "")}
            >
              Expand
            </Button>
          </div>
        </div>
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
        stagedOption={stagedOption}
        onPick={onStage}
        onOther={(choiceId, text) => {
          // Typing in Other IS the staging — no button (rc3 D35). Emptying the
          // box withdraws the custom answer.
          if (text.trim()) onStage(choiceId, text.trim());
          else onUnstage?.();
        }}
      />
      {stagedOption !== undefined && (
        <div className="mt-1 flex items-center gap-2 text-[0.7rem] text-on-surface-variant">
          <span className="rounded bg-primary/15 px-1.5 py-0.5 text-primary">
            ✓ staged: {stagedOption}
          </span>
          <span>sends with your message</span>
          {onUnstage && (
            <button
              type="button"
              onClick={onUnstage}
              className="underline underline-offset-2 hover:text-on-surface"
            >
              undo
            </button>
          )}
        </div>
      )}
    </div>
  );
}
