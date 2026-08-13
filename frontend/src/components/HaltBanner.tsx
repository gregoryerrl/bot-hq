import { cn } from "../lib/cn";
import { authorColorClass } from "./authorColor";

/** One pending tray row, as `list_session_tray` returns it. */
export type TrayRow = {
  id: number;
  agent: string;
  kind: string;
  prompt: string;
  status: string;
  asked_at: string;
};

/**
 * **Why the session has stopped, above the box where you answer it.**
 *
 * A halt is the session saying it needs the user. Until now it said so in the
 * tray — a list you switch to, alongside every question ever asked and answered
 * — while the input box sat empty with no explanation. The whole arc that
 * produced this banner was the user asking "why is it stopped?" and the answer
 * requiring six queries across two tables and a log.
 *
 * Three properties, and each one is a bug this replaces:
 *
 * 1. **One slot.** Halts used to accumulate: answering a tray card released the
 *    ring but left its row pending (rc3 D28), so the bell stayed lit, the user
 *    answered again, and the agent parked another. Measured across the archive:
 *    52 occasions where a second row opened while the first was unanswered, the
 *    worst one row sitting under six more for 53 minutes. D28 fixed the cause;
 *    this makes a recurrence visible immediately, because a stale halt would be
 *    a sentence contradicting a session you can watch working.
 *
 * 2. **It says what it is waiting FOR.** The agent knows, at the moment it
 *    stops. That knowledge used to go nowhere the user would look.
 *
 * 3. **It is adjacent to the answer.** The user's own case: a halt asking for
 *    the output of `php tinker …`, and the paste that satisfies it typed one
 *    line below. Read and answer without switching context.
 *
 * The tray keeps structured picks — a multiple-choice question is a click, not
 * a sentence. The rule is: this banner is the session's STATE, always present
 * while halted; a tray choice is an optional pick attached to it.
 */
export function HaltBanner({
  rows,
  label,
  onOpenTray,
}: {
  /** Every tray row for this session; this component does the filtering. */
  rows: readonly TrayRow[];
  /** slug → what to print for it (rc3 D10/D20), so the banner names a
   *  participant exactly as the chat byline does. */
  label?: (agent: string) => string;
  /** Jump to the tray, when a pick is waiting there too. */
  onOpenTray?: () => void;
}) {
  const halts = rows.filter((r) => r.kind === "halt" && r.status === "pending");
  const choices = rows.filter(
    (r) => r.kind !== "halt" && r.status === "pending",
  );
  // Not halted: render nothing at all rather than an empty bar. A banner that is
  // always present stops being read.
  if (halts.length === 0 && choices.length === 0) return null;

  return (
    <div
      role="status"
      aria-label="Session halted"
      className="border-b border-outline-variant bg-surface-container-low px-3 py-2"
    >
      <div className="flex items-baseline gap-2">
        <span className="font-label-caps text-label-caps text-primary">
          ⏸ HALT
        </span>
        <span className="text-xs text-on-surface-variant">
          {halts.length > 0
            ? "the session is waiting on you"
            : "a question is waiting in the tray"}
        </span>
      </div>
      {/* One line per blocked participant. Multiple is reachable since rc3 D22:
          a park no longer stops the ring where it stands, so the rotation
          finishes its lap and a second participant can park before it yields. */}
      <ul className="mt-1 space-y-1">
        {halts.map((h) => {
          const who = label?.(h.agent) ?? h.agent;
          return (
            <li key={h.id} className="text-sm text-on-surface">
              <span className={cn("font-semibold", authorColorClass(who))}>
                {who}
              </span>
              <span className="text-on-surface-variant"> — </span>
              <span>{h.prompt}</span>
            </li>
          );
        })}
      </ul>
      {choices.length > 0 && (
        <button
          type="button"
          onClick={onOpenTray}
          className="mt-1 text-xs text-primary underline underline-offset-2"
        >
          {choices.length} question{choices.length > 1 ? "s" : ""} waiting in the
          tray →
        </button>
      )}
      <p className="mt-1 text-[0.7rem] text-on-surface-variant">
        Answering — here or in the tray — clears this and restarts the session.
      </p>
    </div>
  );
}
