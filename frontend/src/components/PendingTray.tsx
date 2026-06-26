import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useTauriQuery } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import { BellIcon } from "./icons";

// `list_pending_tray` returns durable pending session_tray rows for open
// sessions. Typed locally rather than via the generated `SessionTrayView`
// binding: the binding only regenerates at app launch, and the notifier just
// needs the session id to group by. Raw invoke works without a bindings entry.
interface PendingTrayRow {
  session_id: string;
}

/**
 * Topbar NOTIFIER for pending input across all open sessions — questions,
 * approval-gates, gated commands. Reads the durable `session_tray` (via
 * `list_pending_tray`) so it reflects input that piled up while the user was
 * AFK and survives a restart, unlike the in-memory pending map. Grouped by
 * session: one row per session ("Session-X needs your input [N]"). Notify-only
 * (per #7) — it links to the session; answering happens on that session's Tray
 * tab. Badge counts sessions awaiting + pulses when non-empty.
 */
export function PendingTray() {
  const [open, setOpen] = useState(false);
  // Durable source (pending session_tray rows for open sessions) so the
  // notifier reflects input that accumulated while AFK AND survives a restart,
  // unlike the in-memory list_pending_choices.
  const { data: pending = [] } = useTauriQuery<PendingTrayRow[]>(
    "list_pending_tray",
    {},
  );

  // Group pending by session so the notifier reads "Session-X needs your input
  // [N]" instead of one row per item. The bell badge counts SESSIONS awaiting,
  // not raw items. Stays notify-only — answering happens on that session's Tray
  // tab; the CTA here is just "go to session".
  const bySession = new Map<string, number>();
  for (const q of pending) {
    bySession.set(q.session_id, (bySession.get(q.session_id) ?? 0) + 1);
  }
  const sessions = [...bySession.entries()];
  const count = sessions.length;

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
        aria-label={`Notifications (${count} session${count === 1 ? "" : "s"} need input)`}
        title={
          count === 0
            ? "Notifications — nothing awaiting"
            : `Notifications — ${count} session${count === 1 ? "" : "s"} need your input`
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
            Awaiting your input — {count} session{count === 1 ? "" : "s"}
          </header>
          {count === 0 ? (
            <p className="px-3 py-4 font-body-md text-body-md text-on-surface-variant">
              All clear.
            </p>
          ) : (
            sessions.map(([sid, n]) => (
              <Link
                key={sid}
                to={`/sessions/${sid}`}
                onClick={() => setOpen(false)}
                className="block border-b border-outline-variant px-3 py-3 last:border-b-0 hover:bg-surface-container-high"
              >
                <div className="mb-1 flex items-center justify-between font-label-caps text-label-caps text-on-surface-variant">
                  <span>Session {sid.slice(0, 8)}</span>
                  <span className="text-primary">Open →</span>
                </div>
                <p className="font-body-md text-body-md text-on-surface">
                  needs your input{" "}
                  <span className="font-semibold text-primary">[{n}]</span>
                </p>
              </Link>
            ))
          )}
        </div>
      )}
    </div>
  );
}

