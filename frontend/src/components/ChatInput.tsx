import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button } from "./ui/Button";
import { Textarea } from "./ui/Textarea";
import { errorMessage } from "../hooks/useInvoke";
import { isLocked, type SessionActivity } from "../stores/activity";

interface ChatInputProps {
  placeholder?: string;
  onSend: (text: string) => Promise<void> | void;
  disabled?: boolean;
  /**
   * The session's duo activity. When `busy`/`cancelling` the input locks and
   * the Send button becomes a Stop button (interrupt redesign, Batch 4).
   * `idle`/`awaiting_user`/undefined leave the input open.
   */
  activity?: SessionActivity;
  /** Hard-cancel the in-flight turn (the Stop button). Required for Stop to
   *  render — without it a locked input just disables, no Stop. */
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

  // The duo is working → lock the textarea + swap Send for Stop.
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
      <form onSubmit={handleSubmit} className="flex items-end gap-2 p-3">
      <div className="relative flex-1">
        <Textarea
          ref={textareaRef}
          rows={2}
          placeholder={placeholder ?? "Message…"}
          value={value}
          onChange={(e) => updateValue(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends; Shift+Enter inserts a newline (so multi-line messages
            // aren't lost). ⌘/Ctrl+Enter also sends. Skip while an IME is
            // composing so multibyte input isn't cut off mid-character.
            if (
              e.key === "Enter" &&
              !e.shiftKey &&
              !e.nativeEvent.isComposing
            ) {
              e.preventDefault();
              handleSubmit(e as unknown as FormEvent);
            }
          }}
          disabled={disabled || sending || locked}
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
      {locked && onCancel ? (
        <Button
          type="button"
          variant="danger"
          onClick={handleCancel}
          disabled={cancelling}
          // Match Send's width so swapping Send↔Stop doesn't shift the layout.
          className="min-w-[5.5rem]"
          title="Stop the in-flight turn"
        >
          {cancelling ? "Cancelling…" : "Stop"}
        </Button>
      ) : (
        <Button
          type="submit"
          variant="primary"
          disabled={!value.trim() || disabled || sending || locked}
          // Fixed min-width so the label cycle (Send → Sending… → Send)
          // doesn't dance the layout on every submit.
          className="min-w-[5.5rem]"
        >
          {sending ? "Sending…" : "Send"}
        </Button>
      )}
      </form>
    </>
  );
}
