import { useEffect, useRef, useState, type FormEvent } from "react";
import { Button } from "./ui/Button";
import { Textarea } from "./ui/Textarea";

interface ChatInputProps {
  placeholder?: string;
  onSend: (text: string) => Promise<void> | void;
  disabled?: boolean;
}

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

  return (
    <form onSubmit={handleSubmit} className="flex items-end gap-2 p-3">
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
        className="flex-1 resize-none"
      />
      <Button
        type="submit"
        variant="primary"
        disabled={!value.trim() || disabled || sending}
      >
        {sending ? "Sending…" : "Send"}
      </Button>
    </form>
  );
}
