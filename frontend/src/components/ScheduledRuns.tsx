import { useEffect, useState } from "react";
import { Button } from "./ui/Button";
import { cn } from "../lib/cn";
import { authorColorClass } from "./authorColor";
import { parseUtcMs } from "../lib/time";
import type { TrayRow } from "./HaltBanner";
import { errorMessage } from "../hooks/useInvoke";
import { ConfirmDialog } from "./ConfirmDialog";
import { ClockIcon } from "./icons";

/**
 * The approved gates still waiting for their run window to open (feedback
 * #66a): the user approved early, bot-hq runs the command at the window's
 * start, unattended — and until then the user can take the approval back.
 * One line per gate: when it runs, in how long, the command's first line, and
 * Cancel. Rendered above the halt banner; empty when nothing is scheduled.
 */
export function ScheduledRuns({
  rows,
  label,
  hues,
  onCancel,
}: {
  rows: readonly TrayRow[];
  label?: (agent: string) => string;
  hues?: Record<string, string>;
  onCancel: (choiceId: string) => Promise<void>;
}) {
  const scheduled = rows.filter((r) => r.scheduled);
  // A clock so "in 4 min" counts down without a refetch.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (scheduled.length === 0) return;
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, [scheduled.length]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The row whose Cancel is awaiting the confirm — taking an approval back is
  // one click away from a running command's output, so it asks first.
  const [confirming, setConfirming] = useState<TrayRow | null>(null);
  if (scheduled.length === 0) return null;

  const cancel = async (row: TrayRow) => {
    setConfirming(null);
    setBusy(row.choice_id);
    setError(null);
    try {
      await onCancel(row.choice_id);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div
      role="group"
      aria-label="Scheduled runs"
      data-testid="scheduled-runs"
      className="border-t border-tertiary/40 bg-surface-container-low px-3 py-2"
    >
      {scheduled.map((row) => {
        const who = label?.(row.agent) ?? row.agent;
        const command = firstLine(row.command_text ?? "");
        return (
          <div
            key={row.choice_id}
            className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 py-0.5 font-code-sm text-code-sm"
          >
            <span className="inline-flex items-center gap-1 font-label-caps text-label-caps text-tertiary">
              <ClockIcon className="size-3" />
              SCHEDULED
            </span>
            <span className={cn("font-semibold", authorColorClass(who, hues))}>{who}</span>
            <span className="text-on-surface-variant">{runsLabel(row.run_not_before ?? null, now)}</span>
            {/* The first line, clipped at the pane's width; the whole command
                is in the title and in the gate ledger. */}
            <code
              className="min-w-[8rem] flex-1 basis-40 truncate font-code-sm text-on-surface"
              title={row.command_text ?? ""}
            >
              {command}
            </code>
            <Button
              size="sm"
              variant="ghost"
              className="ml-auto"
              disabled={busy === row.choice_id}
              onClick={() => setConfirming(row)}
            >
              {busy === row.choice_id ? "Cancelling…" : "Cancel run"}
            </Button>
          </div>
        );
      })}
      {error && <p className="mt-1 font-code-sm text-code-sm text-error">{error}</p>}
      <ConfirmDialog
        open={confirming !== null}
        title="Cancel this scheduled run?"
        message={
          confirming ? (
            <>
              <p>
                The approval is taken back and the command will not run. The agent is told the
                run was cancelled.
              </p>
              <pre className="mt-2 max-h-40 overflow-y-auto overflow-x-hidden whitespace-pre-wrap break-all rounded border border-outline-variant bg-surface-container-low px-2 py-1.5">
                {confirming.command_text ?? ""}
              </pre>
            </>
          ) : null
        }
        confirmLabel="Cancel run"
        cancelLabel="Keep it scheduled"
        confirmVariant="danger"
        onConfirm={() => {
          if (confirming) void cancel(confirming);
        }}
        onCancel={() => setConfirming(null)}
      />
    </div>
  );
}

function firstLine(command: string): string {
  return command.split("\n").find((l) => l.trim() !== "")?.trim() ?? "";
}

/** `runs 09:18 (in 4 min)` on the user's clock; `runs now` once due. */
export function runsLabel(notBefore: string | null, nowMs: number): string {
  if (!notBefore) return "runs when its window opens";
  const at = parseUtcMs(notBefore);
  if (Number.isNaN(at)) return "runs when its window opens";
  const t = new Date(at);
  const hh = String(t.getHours()).padStart(2, "0");
  const mm = String(t.getMinutes()).padStart(2, "0");
  const diff = Math.round((at - nowMs) / 60_000);
  if (diff <= 0) return `runs now (${hh}:${mm})`;
  if (diff < 60) return `runs ${hh}:${mm} (in ${diff} min)`;
  const h = Math.floor(diff / 60);
  return `runs ${hh}:${mm} (in ${h} h ${diff - h * 60} min)`;
}
