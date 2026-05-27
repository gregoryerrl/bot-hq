import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "react-router-dom";
import { useTauriQuery } from "../hooks/useInvoke";
import { Button } from "./ui/Button";
import { cn } from "../lib/cn";
import type { PendingChoiceView } from "../lib/bindings";

/**
 * Aggregate view of all parked questions across sessions. A single tray so
 * a question from any session is reachable in one click from anywhere in
 * the app. Topbar visibility: always present (no longer hidden behind a
 * low-contrast neutral-400 text on the navy surface), with a count badge
 * + pulsing primary tint when something is awaiting.
 */
export function PendingTray() {
  const [open, setOpen] = useState(false);
  const { data: pending = [], refetch } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
    {},
    { refetchInterval: 2_000 },
  );

  const count = pending.length;

  // Click outside + Escape to dismiss. The tray sits in a fixed topbar so
  // a global keydown is fine; click-outside uses a ref + capture-phase
  // listener so the button itself doesn't immediately re-close on click.
  const wrapRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onClick = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onClick);
    };
  }, [open]);

  return (
    <div ref={wrapRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Questions tray (${count} pending)`}
        title={
          count === 0
            ? "Questions tray — nothing awaiting"
            : `Questions tray — ${count} awaiting`
        }
        className={cn(
          "relative inline-flex items-center gap-1.5 rounded border px-2 py-1 font-label-caps text-label-caps transition-colors",
          count > 0
            ? "border-primary bg-primary/15 text-primary animate-pulse"
            : "border-outline/40 text-on-surface hover:border-outline hover:text-on-surface",
        )}
      >
        <BellIcon />
        <span>Questions</span>
        {count > 0 && (
          <span className="ml-1 inline-flex min-w-[1.25rem] items-center justify-center rounded-full bg-primary px-1 text-[0.65rem] font-semibold text-on-primary">
            {count}
          </span>
        )}
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Pending questions"
          className="absolute right-0 top-full z-40 mt-1 max-h-[60vh] w-96 overflow-auto rounded-lg border border-default bg-surface-container shadow-2xl"
        >
          <header className="border-b border-default px-3 py-2 font-label-caps text-label-caps text-on-surface-variant">
            Pending questions ({count})
          </header>
          {count === 0 ? (
            <p className="px-3 py-4 font-body-md text-body-md text-on-surface-variant">
              All clear.
            </p>
          ) : (
            pending.map((q) => (
              <div
                key={q.choice_id}
                className="border-b border-default px-3 py-3 last:border-b-0"
              >
                <div className="mb-1 flex items-center justify-between font-label-caps text-label-caps text-on-surface-variant">
                  <Link
                    to={`/sessions/${q.session_id}`}
                    onClick={() => setOpen(false)}
                    className="hover:text-on-surface"
                  >
                    {q.agent} · {q.session_id.slice(0, 8)}
                  </Link>
                </div>
                <p className="font-body-md text-body-md text-on-surface">
                  {q.question}
                </p>
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

function BellIcon() {
  return (
    <svg
      aria-hidden
      viewBox="0 0 16 16"
      width="14"
      height="14"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M8 1.5a3.5 3.5 0 0 0-3.5 3.5v2.086c0 .31-.123.608-.343.828L3 9.07v.43h10v-.43l-1.157-1.157a1.17 1.17 0 0 1-.343-.828V5A3.5 3.5 0 0 0 8 1.5Z" />
      <path d="M6.5 12a1.5 1.5 0 0 0 3 0" />
    </svg>
  );
}
