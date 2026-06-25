import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button } from "./ui/Button";
import { Textarea } from "./ui/Textarea";
import { errorMessage } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import { isLocked, type DuoBusy, type SessionActivity } from "../stores/activity";

interface ChatInputProps {
  placeholder?: string;
  onSend: (text: string) => Promise<void> | void;
  disabled?: boolean;
  /**
   * The session's duo activity. While `busy`/`cancelling` the textarea is
   * REPLACED by a turn-status line (which agent is working) + the Stop button —
   * the user stops the turn to reclaim the input, then types. `idle` /
   * `awaiting_user` show the normal textarea + Send.
   */
  activity?: SessionActivity;
  /** Per-agent busy flags, for the turn-status line. The collapsed `activity`
   *  says "someone is busy"; this says who (Brian working / Rain reviewing). */
  busy?: DuoBusy;
  /** Hard-cancel the in-flight turn (the Stop button). Without it a locked
   *  session shows the status line but no Stop. */
  onCancel?: () => Promise<void> | void;
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
  onCancel,
  draftKey,
}: ChatInputProps) {
  const [value, setValue] = useState(() =>
    draftKey ? (localStorage.getItem(draftKey) ?? "") : "",
  );
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // The duo is working (busy/cancelling). While locked we hide the textarea and
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
        <div
          role="alert"
          className="mx-3 mt-2 rounded border border-error/40 bg-error-container/30 px-3 py-1.5 text-xs text-on-error-container"
        >
          <span className="font-semibold">Send failed:</span> {error}
          <button
            type="button"
            className="ml-2 underline hover:text-error"
            onClick={() => setError(null)}
          >
            dismiss
          </button>
        </div>
      )}
      <form
        onSubmit={handleSubmit}
        className={cn("flex gap-2 p-3", locked ? "items-center" : "items-end")}
      >
        {locked ? (
          <>
            <TurnStatus activity={activity} busy={busy} />
            {onCancel && (
              <Button
                type="button"
                variant="danger"
                onClick={handleCancel}
                // Disabled while the cancel is in flight — either the local press
                // latency (`cancelling`) or the backend's explicit `cancelling`.
                disabled={cancelling || activity === "cancelling"}
                className="min-w-[5.5rem]"
                title="Stop the in-flight turn"
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

// Shown in place of the textarea while the duo is working: which agent is doing
// what, with a little animated spice. The user Stops the turn to reclaim the
// input. Brian (HANDS) = orange/primary "working"; Rain (EYES) = purple/
// secondary "reviewing"; a broadcast can have both busy at once.
function TurnStatus({
  activity,
  busy,
}: {
  activity?: SessionActivity;
  busy?: DuoBusy;
}) {
  // A cancel-in-flight reads as "Stopping…" regardless of who was busy.
  if (activity === "cancelling") {
    return (
      <div className="flex flex-1 items-center gap-2 px-1 text-xs text-on-surface-variant">
        <span className="animate-pulse">Stopping the turn…</span>
      </div>
    );
  }
  const workers: { name: string; verb: string; color: string }[] = [];
  if (busy?.brian)
    workers.push({ name: "Brian", verb: "working", color: "text-primary" });
  if (busy?.rain)
    workers.push({ name: "Rain", verb: "reviewing", color: "text-secondary" });
  return (
    <div className="flex flex-1 items-center gap-2 px-1 text-xs text-on-surface-variant">
      <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
        {workers.length === 0 ? (
          // Locked but no per-agent flag yet (e.g. a stale snapshot): stay generic.
          <span>The duo is working</span>
        ) : (
          workers.map((w, i) => (
            <span key={w.name} className="flex items-center gap-1.5">
              {i > 0 && <span className="text-on-surface-variant/40">·</span>}
              <span className={cn("font-semibold", w.color)}>{w.name}</span>
              <span>is {w.verb}</span>
            </span>
          ))
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
