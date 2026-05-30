import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useTauriQuery } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import type { PendingChoiceView } from "../lib/bindings";

/**
 * Topbar NOTIFIER for parked questions AND approval-gates across all sessions
 * (both arrive as PendingChoices). This is a
 * notify-only surface (per #7): it tells the user which sessions need input
 * and links straight to them — it does NOT answer questions inline. Answering
 * happens in the session chat's questions tray (the sole answer surface), so
 * the user always sees the full conversation context (and the mandatory
 * "Other" free-text option) when they decide. Count badge + pulsing primary
 * tint when something is awaiting.
 */
export function PendingTray() {
  const [open, setOpen] = useState(false);
  const { data: pending = [] } = useTauriQuery<PendingChoiceView[]>(
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
        aria-label={`Notifications (${count} awaiting: questions & approvals)`}
        title={
          count === 0
            ? "Notifications — nothing awaiting"
            : `Notifications — ${count} awaiting (questions & approval gates)`
        }
        className={cn(
          "relative inline-flex items-center rounded border p-1.5 transition-colors",
          count > 0
            ? "border-primary bg-primary/15 text-primary animate-pulse"
            : "border-outline/40 text-on-surface hover:border-outline hover:text-on-surface",
        )}
      >
        <BellIcon />
        {count > 0 && (
          <span className="absolute -right-1.5 -top-1.5 inline-flex min-w-[1.25rem] items-center justify-center rounded-full bg-primary px-1 text-[0.65rem] font-semibold text-on-primary">
            {count}
          </span>
        )}
      </button>
      {open && (
        <div
          role="dialog"
          aria-label="Pending notifications"
          className="absolute right-0 top-full z-40 mt-1 max-h-[60vh] w-96 overflow-auto rounded-lg border border-outline-variant bg-surface-container shadow-2xl"
        >
          <header className="border-b border-outline-variant px-3 py-2 font-label-caps text-label-caps text-on-surface-variant">
            Awaiting your input ({count})
          </header>
          {count === 0 ? (
            <p className="px-3 py-4 font-body-md text-body-md text-on-surface-variant">
              All clear.
            </p>
          ) : (
            pending.map((q) => (
              <Link
                key={q.choice_id}
                to={`/sessions/${q.session_id}`}
                onClick={() => setOpen(false)}
                className="block border-b border-outline-variant px-3 py-3 last:border-b-0 hover:bg-surface-container-high"
              >
                <div className="mb-1 flex items-center justify-between font-label-caps text-label-caps text-on-surface-variant">
                  <span>
                    {q.agent} · {q.session_id.slice(0, 8)}
                  </span>
                  <span className="text-primary">Open →</span>
                </div>
                <p className="font-body-md text-body-md text-on-surface">
                  {q.question}
                </p>
              </Link>
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
      width="16"
      height="16"
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
