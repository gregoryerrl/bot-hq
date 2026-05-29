import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { Button } from "./ui/Button";
import { Textarea } from "./ui/Textarea";

interface ChatInputProps {
  placeholder?: string;
  onSend: (text: string) => Promise<void> | void;
  disabled?: boolean;
}

// macOS uses ⌘; everywhere else show "Ctrl". Detection is best-effort:
// `navigator.platform` is deprecated but still populated; falling back to
// userAgent keeps the hint correct on common platforms without pulling in a
// dep. Computed once at module-load — no need to track changes.
const isMac =
  typeof navigator !== "undefined" &&
  (/Mac|iPhone|iPad/i.test(navigator.platform) ||
    /Mac|iPhone|iPad/i.test(navigator.userAgent));
const modKeyLabel = isMac ? "⌘" : "Ctrl";

export function ChatInput({ placeholder, onSend, disabled }: ChatInputProps) {
  const [value, setValue] = useState("");
  const [sending, setSending] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

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
    try {
      await onSend(text);
      setValue("");
    } finally {
      setSending(false);
    }
  };

  // Stable identity so the hint <kbd> elements don't reflow on every render.
  const hint = useMemo(() => `${modKeyLabel}↵`, []);

  return (
    <form onSubmit={handleSubmit} className="flex items-end gap-2 p-3">
      <div className="relative flex-1">
        <Textarea
          ref={textareaRef}
          rows={2}
          placeholder={placeholder ?? "Message…"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
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
          title="Send"
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
    </form>
  );
}
