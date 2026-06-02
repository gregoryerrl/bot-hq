import { useEffect, useState } from "react";
import { useTauriQuery } from "../hooks/useInvoke";
import { PhasePillRow, type Phase } from "./PhasePill";
import { Markdown } from "./Markdown";
import { cn } from "../lib/cn";
import type { SessionDocumentView, SessionTrayView } from "../lib/bindings";

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

interface DiffLine {
  kind: string;
  text: string;
}

interface ComputeApplyDiffResult {
  lines: DiffLine[];
  note: string | null;
}

export function DocumentPane({ sessionId, sessionPhase }: DocumentPaneProps) {
  const [activePhase, setActivePhase] = useState<Phase>(
    sessionPhase ?? "investigate",
  );
  // The Tray tab sits before I/P/A/V and is phase-independent: it shows the
  // session's durable tray (every question / approval / gated command —
  // pending + resolved history). A phase transition updates the underlying
  // phase but does NOT yank the user off the Tray.
  const [showTray, setShowTray] = useState(false);

  // Follow the session's phase whenever it changes (the fix for #3). Firing
  // only on sessionPhase change means a manual tab click still sticks until
  // the next phase transition, so the user can freely peek at other phases.
  useEffect(() => {
    if (sessionPhase) setActivePhase(sessionPhase);
  }, [sessionPhase]);

  const { data: docs = [] } = useTauriQuery<SessionDocumentView[]>(
    "session_doc_search",
    { sessionId, phase: activePhase },
  );

  // Apply tab gets a live git diff above the phase=apply session docs.
  // Refetches on session change; Apply tab visibility doesn't matter for the
  // query — TanStack Query caches it for instant switch-back.
  const { data: applyDiff } = useTauriQuery<ComputeApplyDiffResult>(
    "compute_apply_diff",
    { sessionId },
    { enabled: !!sessionId && activePhase === "apply" },
  );

  const activeDocs = docs.filter((d) => d.phase === activePhase);

  return (
    <div className="flex h-full min-w-0 flex-col border-l border-outline-variant bg-surface-container-lowest/50">
      <div className="flex items-center gap-1 border-b border-outline-variant px-3 py-2">
        <TrayPill selected={showTray} onSelect={() => setShowTray(true)} />
        <span className="mx-1 h-4 w-px bg-outline-variant" aria-hidden />
        <PhasePillRow
          selected={showTray ? null : activePhase}
          onSelect={(p) => {
            setShowTray(false);
            setActivePhase(p);
          }}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
        {showTray ? (
          <TrayList sessionId={sessionId} />
        ) : (
          <>
            {activePhase === "apply" && (
              <ApplyDiffBlock diff={applyDiff ?? null} />
            )}
            {activeDocs.length === 0 ? (
              activePhase === "apply" && applyDiff?.lines.length ? null : (
                <p className="text-sm text-on-surface-variant">
                  No {activePhase} documents yet.
                </p>
              )
            ) : (
              activeDocs.map((doc) => (
                <article key={doc.id} className="mb-6">
                  <header className="mb-2 flex items-center justify-between">
                    <h4 className="text-sm font-semibold text-on-surface">
                      {doc.slug}
                    </h4>
                    <span className="text-[0.65rem] text-on-surface-variant">
                      {doc.updated_at}
                    </span>
                  </header>
                  <Markdown>{doc.body}</Markdown>
                </article>
              ))
            )}
          </>
        )}
      </div>
    </div>
  );
}

// GitHub dark-mode palette: green adds, red removes, blue hunk headers,
// yellow file headers, muted context. Mirrors the Rust diff classifier
// `parse_diff_lines` in `tauri_cmd/docs.rs`.
const diffLineClass: Record<string, string> = {
  add: "bg-emerald-500/10 text-emerald-300",
  remove: "bg-red-500/10 text-red-300",
  hunk: "bg-blue-500/10 text-blue-300",
  file: "text-amber-300",
  context: "text-neutral-400",
};

function ApplyDiffBlock({ diff }: { diff: ComputeApplyDiffResult | null }) {
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
  return (
    <section className="mb-6 rounded border border-outline-variant bg-surface-container-lowest">
      <header className="border-b border-outline-variant px-3 py-1.5 text-[0.65rem] uppercase tracking-wide text-on-surface-variant">
        git diff{diff.note ? ` — ${diff.note}` : ""}
      </header>
      <pre className="overflow-x-auto px-3 py-2 text-[0.7rem] leading-relaxed font-mono">
        {diff.lines.map((line, i) => (
          <div
            key={i}
            className={cn(
              "whitespace-pre-wrap",
              diffLineClass[line.kind] ?? "text-neutral-300",
            )}
          >
            {line.text || " "}
          </div>
        ))}
      </pre>
    </section>
  );
}

// ---- Tray tab ----------------------------------------------------------

function TrayPill({
  selected,
  onSelect,
}: {
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      onClick={onSelect}
      className={cn(
        "inline-flex items-center rounded px-2 py-1 text-xs font-semibold uppercase border-t-2",
        selected
          ? "border-on-surface/70 bg-surface-container-high/80 text-on-surface"
          : "border-transparent bg-transparent text-on-surface-variant hover:text-on-surface",
      )}
      title="Session tray — pending questions, approvals & gated commands (and resolved history)"
    >
      Tray
    </button>
  );
}

const trayStatusClass: Record<string, string> = {
  pending: "border-primary/70 text-primary",
  answered: "border-tertiary/60 text-tertiary",
  withdrawn: "border-outline-variant text-on-surface-variant",
  superseded: "border-outline-variant text-on-surface-variant",
};

function TrayList({ sessionId }: { sessionId: string }) {
  const { data: entries = [] } = useTauriQuery<SessionTrayView[]>(
    "list_session_tray",
    { sessionId },
  );
  if (entries.length === 0) {
    return (
      <p className="text-sm text-on-surface-variant">
        Tray is empty — no questions, approvals, or gated commands yet.
      </p>
    );
  }
  // Pending first, then resolved most-recent-first (rows arrive oldest-first).
  const pending = entries.filter((e) => e.status === "pending");
  const resolved = entries.filter((e) => e.status !== "pending").reverse();
  return (
    <ul className="space-y-3">
      {[...pending, ...resolved].map((e) => (
        <TrayItem key={e.id} entry={e} />
      ))}
    </ul>
  );
}

function TrayItem({ entry }: { entry: SessionTrayView }) {
  const isPending = entry.status === "pending";
  return (
    <li
      className={cn(
        "rounded border bg-surface-container-lowest px-3 py-2",
        isPending ? "border-primary/50" : "border-outline-variant",
      )}
    >
      <div className="mb-1 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="rounded bg-surface-container-high px-1.5 py-0.5 text-[0.6rem] uppercase tracking-wide text-on-surface-variant">
            {entry.kind}
          </span>
          <span className="text-[0.7rem] text-on-surface-variant">
            {entry.agent}
          </span>
        </div>
        <span
          className={cn(
            "rounded border-t-2 px-1.5 py-0.5 text-[0.6rem] font-semibold uppercase",
            trayStatusClass[entry.status] ??
              "border-outline-variant text-on-surface-variant",
          )}
        >
          {entry.status}
        </span>
      </div>
      <p className="text-sm text-on-surface">{entry.prompt}</p>
      {entry.command_text && (
        <pre className="mt-1 overflow-x-auto rounded bg-surface-container-high px-2 py-1 text-[0.7rem] font-mono text-on-surface-variant">
          {entry.command_text}
        </pre>
      )}
      {entry.options.length > 0 && (
        <div className="mt-1 flex flex-wrap gap-1">
          {entry.options.map((opt) => (
            <span
              key={opt}
              className={cn(
                "rounded px-1.5 py-0.5 text-[0.65rem]",
                entry.picked_option === opt
                  ? "bg-primary/20 text-primary"
                  : "bg-surface-container-high text-on-surface-variant",
              )}
            >
              {opt}
              {entry.picked_option === opt ? " ✓" : ""}
            </span>
          ))}
        </div>
      )}
      <div className="mt-1 text-[0.6rem] text-on-surface-variant">
        {entry.asked_at}
        {entry.answered_at ? ` → ${entry.answered_at}` : ""}
      </div>
    </li>
  );
}
