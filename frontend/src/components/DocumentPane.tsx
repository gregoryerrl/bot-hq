import { useEffect, useMemo, useState } from "react";
import { useTauriQuery } from "../hooks/useInvoke";
import { PhasePillRow, type Phase } from "./PhasePill";
import { cn } from "../lib/cn";
import type { SessionDocumentView } from "../lib/bindings";

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

  const counts = useMemo(() => {
    const c: Partial<Record<Phase, number>> = {};
    for (const d of docs) {
      if (d.phase) c[d.phase as Phase] = (c[d.phase as Phase] ?? 0) + 1;
    }
    return c;
  }, [docs]);

  const activeDocs = docs.filter((d) => d.phase === activePhase);

  return (
    <div className="flex h-full min-w-0 flex-col border-l border-neutral-800 bg-neutral-950/50">
      <div className="flex items-center justify-between gap-2 border-b border-neutral-800 px-3 py-2">
        <PhasePillRow
          selected={activePhase}
          onSelect={setActivePhase}
          counts={counts}
        />
        <span className="text-[0.65rem] uppercase tracking-wide text-neutral-500">
          {activeDocs.length} doc{activeDocs.length === 1 ? "" : "s"}
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
        {activePhase === "apply" && (
          <ApplyDiffBlock diff={applyDiff ?? null} />
        )}
        {activeDocs.length === 0 ? (
          activePhase === "apply" && applyDiff?.lines.length ? null : (
            <p className="text-sm text-neutral-500">
              No {activePhase} documents yet.
            </p>
          )
        ) : (
          activeDocs.map((doc) => (
            <article key={doc.id} className="mb-6">
              <header className="mb-2 flex items-center justify-between">
                <h4 className="text-sm font-semibold text-neutral-200">
                  {doc.slug}
                </h4>
                <span className="text-[0.65rem] text-neutral-500">
                  {doc.updated_at}
                </span>
              </header>
              <pre className="whitespace-pre-wrap text-xs leading-relaxed text-neutral-300">
                {doc.body}
              </pre>
            </article>
          ))
        )}
      </div>
    </div>
  );
}

// GitHub dark-mode palette: green adds, red removes, blue hunk headers,
// yellow file headers, muted context. Mirrors the Slint-era classifier
// in `view_model::parse_diff_lines`.
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
      <p className="mb-6 text-xs text-neutral-500">Loading diff…</p>
    );
  }
  if (diff.lines.length === 0) {
    return (
      <p className="mb-6 text-xs text-neutral-500">
        {diff.note ?? "(no changes)"}
      </p>
    );
  }
  return (
    <section className="mb-6 rounded border border-neutral-800 bg-neutral-950">
      <header className="border-b border-neutral-800 px-3 py-1.5 text-[0.65rem] uppercase tracking-wide text-neutral-500">
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
