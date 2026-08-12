import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button } from "./ui/Button";
import { Textarea } from "./ui/Textarea";
import { ErrorBanner } from "./ErrorBanner";
import { errorMessage } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import { authorColorClass } from "./authorColor";
import { isLocked, type AgentBusy, type SessionActivity } from "../stores/activity";

interface ChatInputProps {
  placeholder?: string;
  onSend: (text: string) => Promise<void> | void;
  disabled?: boolean;
  /**
   * The session's activity. While `busy`/`cancelling` the textarea is
   * REPLACED by a turn-status line (which participants are working) + Stop —
   * the user stops the turn to reclaim the input, then types. `idle` /
   * `awaiting_user` show the normal textarea + Send.
   */
  activity?: SessionActivity;
  /** Per-participant busy flags, for the turn-status line. The collapsed
   *  `activity` says "someone is busy"; this says which participants. */
  busy?: AgentBusy;
  /** Participant slug -> what to PRINT for it (rc3 D10: `ROLE · Model`, never
   *  an agent name). Without it the status line falls back to the slug. */
  busyLabel?: (slug: string) => string;
  /** Pause the in-flight turn (the Stop button — interrupts the agents and
   *  lands the session in `paused`). Without it a locked session shows the
   *  status line but no Stop. */
  onCancel?: () => Promise<void> | void;
  /** Resume a paused session (the paused bar's Resume button). The backend
   *  releases the latch, nudges the agents, and flushes anything held. */
  onResume?: () => Promise<void> | void;
  /** Open the force-close flow from the paused bar (the parent owns the
   *  confirm dialog — same flow as the header ✕). */
  onClose?: () => void;
  /**
   * localStorage key for draft persistence. When set, the in-progress text
   * survives unmounts (navigating to another session / app restart): seeded
   * on mount, written through on change, cleared on successful send. The
   * parent must remount this component when the key changes (`key={...}`) —
   * the seed is a lazy initializer, not an effect.
   */
  draftKey?: string;
}

export function ChatInput({
  placeholder,
  onSend,
  disabled,
  activity,
  busy,
  busyLabel,
  onCancel,
  onResume,
  onClose,
  draftKey,
}: ChatInputProps) {
  const [value, setValue] = useState(() =>
    draftKey ? (localStorage.getItem(draftKey) ?? "") : "",
  );
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [resuming, setResuming] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // A turn is in flight (busy/cancelling). While locked we hide the textarea and
  // show the turn-status line + Stop, rather than leaving the input typeable.
  const locked = isLocked(activity);
  // Once the turn actually stops (activity leaves busy/cancelling) drop the
  // local "Cancelling…" spinner. v1 has no explicit backend cancelling state
  // (it goes busy → idle), so this is the post-press feedback.
  useEffect(() => {
    if (!locked) setCancelling(false);
  }, [locked]);

  const handleCancel = async () => {
    if (!onCancel || cancelling) return;
    setCancelling(true);
    try {
      await onCancel();
    } catch (err) {
      setError(errorMessage(err));
      setCancelling(false);
    }
  };

  // The session is paused (Stop landed): textarea stays open for a steer, and
  // the paused bar offers Resume / Close.
  const paused = activity === "paused";
  // Drop the local "Resuming…" latch once the backend leaves paused.
  useEffect(() => {
    if (!paused) setResuming(false);
  }, [paused]);

  const handleResume = async () => {
    if (!onResume || resuming) return;
    setResuming(true);
    try {
      await onResume();
    } catch (err) {
      setError(errorMessage(err));
      setResuming(false);
    }
  };

  const updateValue = (next: string) => {
    setValue(next);
    if (!draftKey) return;
    // Drop the key entirely when the box is emptied so abandoned sessions
    // don't accumulate "" entries in localStorage.
    if (next) localStorage.setItem(draftKey, next);
    else localStorage.removeItem(draftKey);
  };

  // Auto-grow: reset to `auto` so scrollHeight reflects actual content height,
  // then clamp to 200px (~8 rows). Beyond that the textarea scrolls
  // internally instead of pushing the chat list off-screen.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [value]);

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const text = value.trim();
    if (!text || disabled || sending) return;
    setSending(true);
    setError(null);
    try {
      await onSend(text);
      updateValue("");
    } catch (err) {
      // Keep `value` so the user can retry without retyping, and surface the
      // failure — a silent reject made the user think the message was sent.
      setError(errorMessage(err));
    } finally {
      setSending(false);
    }
  };

  const hint = "↵";

  return (
    <>
      {error && (
        <ErrorBanner
          label="Send failed:"
          message={error}
          onDismiss={() => setError(null)}
          className="mx-3 mt-2"
        />
      )}
      {paused && (
        <div className="flex items-center gap-2 border-b border-outline-variant bg-surface-container-low px-3 py-2">
          <span className="flex-1 text-xs text-on-surface-variant">
            <span className="font-semibold text-on-surface">⏸ Paused</span>
            {" — agents halted. Type below to steer, or"}
          </span>
          {onResume && (
            <Button
              type="button"
              variant="primary"
              onClick={handleResume}
              disabled={resuming}
              className="min-w-[5.5rem]"
              title="Wake the agents and continue where they left off"
            >
              {resuming ? "Resuming…" : "Resume"}
            </Button>
          )}
          {onClose && (
            <Button
              type="button"
              variant="danger"
              onClick={onClose}
              title="Force-close this session (confirmation follows)"
            >
              Close session
            </Button>
          )}
        </div>
      )}
      {/* Unlocked but still working — the locked branch has its own TurnStatus. */}
      {!locked && (
        <StillWorkingNotice activity={activity} busy={busy} label={busyLabel} />
      )}
      <form
        onSubmit={handleSubmit}
        className={cn("flex gap-2 p-3", locked ? "items-center" : "items-end")}
      >
        {locked ? (
          <>
            <TurnStatus activity={activity} busy={busy} label={busyLabel} />
            {onCancel && (
              <Button
                type="button"
                variant="danger"
                onClick={handleCancel}
                // Disabled while the cancel is in flight — either the local press
                // latency (`cancelling`) or the backend's explicit `cancelling`.
                disabled={cancelling || activity === "cancelling"}
                className="min-w-[5.5rem]"
                title="Pause the agents — the session parks until you steer, resume, or close"
              >
                {cancelling || activity === "cancelling"
                  ? "Cancelling…"
                  : "Stop"}
              </Button>
            )}
          </>
        ) : (
          <>
            <div className="relative flex-1">
              <Textarea
                ref={textareaRef}
                rows={2}
                placeholder={placeholder ?? "Message…"}
                value={value}
                onChange={(e) => updateValue(e.target.value)}
                onKeyDown={(e) => {
                  // Enter sends; Shift+Enter inserts a newline (so multi-line
                  // messages aren't lost). ⌘/Ctrl+Enter also sends. Skip while an
                  // IME is composing so multibyte input isn't cut mid-character.
                  if (
                    e.key === "Enter" &&
                    !e.shiftKey &&
                    !e.nativeEvent.isComposing
                  ) {
                    e.preventDefault();
                    handleSubmit(e as unknown as FormEvent);
                  }
                }}
                disabled={disabled || sending}
                // Right padding leaves room for the kbd hint overlay.
                className="w-full resize-none pr-14"
              />
              <kbd
                aria-hidden
                className="pointer-events-none absolute bottom-1.5 right-2 select-none rounded border border-outline-variant bg-surface-container-lowest px-1.5 py-0.5 font-mono text-[0.65rem] text-on-surface-variant"
                title="Enter to send · Shift+Enter for a newline"
              >
                {hint}
              </kbd>
            </div>
            <Button
              type="submit"
              variant="primary"
              disabled={!value.trim() || disabled || sending}
              // Fixed min-width so the label cycle (Send → Sending… → Send)
              // doesn't dance the layout on every submit.
              className="min-w-[5.5rem]"
            >
              {sending ? "Sending…" : "Send"}
            </Button>
          </>
        )}
      </form>
    </>
  );
}

/** Is any agent mid-turn? Separate from the collapsed `activity`, which can
 *  read `awaiting_user` / `paused` while an agent is still running. */
function anyBusy(busy?: AgentBusy): boolean {
  return Object.values(busy ?? {}).some(Boolean);
}

// Which participants are mid-turn, as a labelled list — a broadcast can have
// every one of them busy at once. Shared by the locked turn-status line and the
// unlocked still-working notice so the two labels can never drift apart.
//
// One verb for everyone. The old line said Brian "is working" and Rain "is
// reviewing", which is bot-hq claiming to know what a role MEANS; it knows only
// that a participant's turn is in flight (rc3 D10/D11). The colour still comes
// from the slug, matching the same author's chat byline.
function WorkerLine({
  busy,
  label,
}: {
  busy?: AgentBusy;
  label?: (slug: string) => string;
}) {
  const workers = Object.entries(busy ?? {})
    .filter(([, isBusy]) => isBusy)
    .map(([slug]) => slug);
  return (
    <>
      {workers.map((slug, i) => (
        <span key={slug} className="flex items-center gap-1.5">
          {i > 0 && <span className="text-on-surface-variant/40">·</span>}
          <span className={cn("font-semibold", authorColorClass(slug))}>
            {label?.(slug) ?? slug}
          </span>
          <span>is working</span>
        </span>
      ))}
    </>
  );
}

/**
 * The input is UNLOCKED but an agent is still mid-turn — the state that reads as
 * "they stopped" and isn't.
 *
 * `SessionActivity::derive` (src/core/activity.rs) ranks `awaiting` ABOVE `busy`
 * on purpose: parking a question must re-open the textarea even though the turn
 * is still in flight, or the user couldn't answer it. But `TurnStatus` only ever
 * rendered inside the locked branch, so the per-agent flags — which the backend
 * emits on EVERY activity event, whatever the derived state — had nowhere to go.
 * The user saw an open input, assumed the work was done, and then watched more
 * output arrive seconds later.
 *
 * The textarea stays enabled here. This line only says the work hasn't stopped.
 */
function StillWorkingNotice({
  activity,
  busy,
  label,
}: {
  activity?: SessionActivity;
  busy?: AgentBusy;
  label?: (slug: string) => string;
}) {
  if (!anyBusy(busy)) return null;
  const paused = activity === "paused";
  return (
    <div className="flex items-center gap-2 border-b border-outline-variant bg-surface-container-low px-3 py-1.5 text-xs text-on-surface-variant">
      <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
        {activity === "awaiting_user" && <span>Waiting on your answer ·</span>}
        {paused && <span>Stopping ·</span>}
        <WorkerLine busy={busy} label={label} />
        <span>
          {paused
            ? "— finishing the current tool."
            : "— the turn hasn't ended yet."}
        </span>
      </span>
      <BouncingDots />
    </div>
  );
}

// Shown in place of the textarea while a turn is in flight: which participants
// are working, with a little animated spice. The user Stops the turn to reclaim
// the input.
function TurnStatus({
  activity,
  busy,
  label,
}: {
  activity?: SessionActivity;
  busy?: AgentBusy;
  label?: (slug: string) => string;
}) {
  // A cancel-in-flight reads as "Stopping…" regardless of who was busy.
  if (activity === "cancelling") {
    return (
      <div className="flex flex-1 items-center gap-2 px-1 text-xs text-on-surface-variant">
        <span className="animate-pulse">Stopping the turn…</span>
      </div>
    );
  }
  return (
    <div className="flex flex-1 items-center gap-2 px-1 text-xs text-on-surface-variant">
      <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
        {anyBusy(busy) ? (
          <WorkerLine busy={busy} label={label} />
        ) : (
          // Locked but no per-agent flag yet (e.g. a stale snapshot): stay generic.
          <span>A participant is working</span>
        )}
      </span>
      <BouncingDots />
    </div>
  );
}

// Three staggered bouncing dots — the "little spice". Decorative; `bg-current`
// inherits the status text colour.
function BouncingDots() {
  return (
    <span className="inline-flex items-end gap-0.5" aria-hidden>
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className="h-1 w-1 animate-bounce rounded-full bg-current"
          style={{ animationDelay: `${i * 150}ms` }}
        />
      ))}
    </span>
  );
}
