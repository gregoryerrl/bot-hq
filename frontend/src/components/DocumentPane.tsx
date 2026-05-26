import { useMemo, useState } from "react";
import { useTauriQuery } from "../hooks/useInvoke";
import { PhasePillRow, type Phase } from "./PhasePill";
import type { SessionDocumentView } from "../lib/bindings";

interface DocumentPaneProps {
  sessionId: string;
}

export function DocumentPane({ sessionId }: DocumentPaneProps) {
  const [activePhase, setActivePhase] = useState<Phase>("investigate");

  const { data: docs = [] } = useTauriQuery<SessionDocumentView[]>(
    "session_doc_search",
    { sessionId, phase: activePhase },
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
    <div className="flex h-full flex-col border-l border-neutral-800 bg-neutral-950/50">
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
      <div className="flex-1 overflow-auto px-4 py-3">
        {activeDocs.length === 0 ? (
          <p className="text-sm text-neutral-500">
            No {activePhase} documents yet.
          </p>
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
