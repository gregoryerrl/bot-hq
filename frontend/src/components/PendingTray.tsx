import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useTauriQuery } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import { BellIcon } from "./icons";
import { needsYou, needsYouBySession, needsYouParts } from "../lib/attention";
import { useNow } from "../hooks/useNow";
import type { OpenSessionHalt } from "../lib/bindings";
import { shortSessionId } from "../lib/sessionId";

// `list_pending_tray` returns durable pending session_tray rows for open
// sessions. Typed locally as the subset this notifier reads (it only groups by
// session id); `Dashboard`/`DocumentPane` type the same command with the
// generated `SessionTrayView` — either works, and this file needs two fields.
interface PendingTrayRow {
  session_id: string;
  kind: string;
  options: string[];
}

/**
 * Topbar NOTIFIER for everything the user is waiting on across open sessions —
 * questions, approval gates and halts, named BY KIND (the user, 2026-09-25:
 * "gates also don't show on notification bell"; this revises rc3 D35, which
 * counted questions only because a blended count lied about an empty tray).
 * Reads the durable `session_tray` (via `list_pending_tray`) and the halt slots
 * (via `list_session_halts`), so it reflects what piled up while the user was
 * AFK and survives a restart. Grouped by session: one row per session
 * ("needs you: approval waiting · 1 question"). Notify-only (per #7) — it links
 * to the session; answering happens there. Badge counts sessions awaiting +
 * pulses when non-empty.
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

  // Every open session's halt, and a clock for temporary halts whose wake
  // time passes without a wake (nothing announces that).
  const { data: haltRows } = useTauriQuery<OpenSessionHalt[] | null>("list_session_halts", {});
  const halts = haltRows ?? [];
  const now = useNow(30_000);

  // Grouped by session: one row per session naming what it waits on, by kind
  // ("approval waiting · halted — your move"). The badge counts SESSIONS, not
  // items. Notify-only — answering happens inside the session.
  const sessions = Object.entries(needsYouBySession(pending, halts, now)).filter(
    ([, n]) => needsYou(n),
  );
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
          className="absolute right-0 top-full z-40 mt-1 max-h-[60vh] w-96 overflow-y-auto overflow-x-hidden rounded-lg border border-outline-variant bg-surface-container shadow-2xl"
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
                  <span>Session {shortSessionId(sid)}</span>
                  <span className="text-primary">Open →</span>
                </div>
                <p className="font-body-md text-body-md text-on-surface">
                  needs you:{" "}
                  <span className="font-semibold text-primary">
                    {needsYouParts(n).join(" · ")}
                  </span>
                </p>
                {n.halt && (
                  <p className="mt-1 line-clamp-2 font-code-sm text-code-sm text-on-surface-variant">
                    {n.halt.reason}
                  </p>
                )}
              </Link>
            ))
          )}
        </div>
      )}
    </div>
  );
}

