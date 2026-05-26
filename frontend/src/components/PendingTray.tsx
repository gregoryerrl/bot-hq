import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "react-router-dom";
import { useTauriQuery } from "../hooks/useInvoke";
import { Button } from "./ui/Button";
import { cn } from "../lib/cn";
import type { PendingChoiceView } from "../lib/bindings";

/**
 * Aggregate view of all parked choices across sessions. Slint-era surfaced
 * pending-input counts as topbar chips per session; the Tauri rebuild needs
 * a single tray so a question from any session is reachable in one click.
 */
export function PendingTray() {
  const [open, setOpen] = useState(false);
  const { data: pending = [], refetch } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
    {},
    { refetchInterval: 2_000 },
  );

  const count = pending.length;

  return (
    <div className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "relative inline-flex items-center gap-1 rounded px-2 py-1 text-xs font-medium",
          count > 0
            ? "text-red-200 hover:bg-red-950/40"
            : "text-neutral-400 hover:bg-neutral-900 hover:text-neutral-100",
        )}
        aria-label={`${count} pending question${count === 1 ? "" : "s"}`}
      >
        <span>Inbox</span>
        {count > 0 && (
          <span className="rounded-full bg-red-500/20 px-1.5 text-[0.65rem] font-semibold text-red-200">
            {count}
          </span>
        )}
      </button>
      {open && (
        <div className="absolute right-0 top-full z-40 mt-1 w-96 max-h-[60vh] overflow-auto rounded-lg border border-neutral-800 bg-neutral-950 shadow-2xl">
          <header className="border-b border-neutral-800 px-3 py-2 text-xs font-semibold uppercase tracking-wide text-neutral-400">
            Pending questions ({count})
          </header>
          {count === 0 ? (
            <p className="px-3 py-4 text-sm text-neutral-500">All clear.</p>
          ) : (
            pending.map((q) => (
              <div
                key={q.choice_id}
                className="border-b border-neutral-800 px-3 py-3 last:border-b-0"
              >
                <div className="mb-1 flex items-center justify-between text-[0.65rem] uppercase tracking-wide text-neutral-500">
                  <Link
                    to={`/sessions/${q.session_id}`}
                    onClick={() => setOpen(false)}
                    className="hover:text-neutral-200"
                  >
                    {q.agent} · {q.session_id.slice(0, 8)}
                  </Link>
                </div>
                <p className="text-sm text-neutral-200">{q.question}</p>
                <div className="mt-2 flex flex-wrap gap-1">
                  {q.options.map((opt) => (
                    <Button
                      key={opt}
                      size="sm"
                      variant="primary"
                      onClick={async () => {
                        await invoke("resolve_choice", {
                          choiceId: q.choice_id,
                          picked: opt,
                        });
                        refetch();
                      }}
                    >
                      {opt}
                    </Button>
                  ))}
                </div>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
