import { useEffect, useMemo, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useStickyScroll } from "../hooks/useStickyScroll";
import { useEmmaStore } from "../stores/emma";
import { useChatStore } from "../stores/chat";
import type {
  AgentMessage,
  AppError,
  PendingChoiceView,
} from "../lib/bindings";
import { cn } from "../lib/cn";
import { formatClockTime } from "../lib/time";

const EMMA_SESSION_ID = "emma";
// Stable reference so the zustand selector doesn't return a fresh `[]` per
// call — `Object.is([], [])` is false, which would trigger infinite re-renders.
const EMPTY_MESSAGES: AgentMessage[] = [];

export function EmmaOverlay() {
  const open = useEmmaStore((s) => s.open);
  const setOpen = useEmmaStore((s) => s.setOpen);

  // Respawn Emma on overlay open. Mirrors SessionView's pattern: when the
  // user closes + reopens bot-hq the Emma subprocess is dead, but the row
  // persists with `brian_claude_session_id` / `rain_claude_session_id`.
  // `ensure_session_started` reads those + passes `--resume <uuid>` so the
  // agent comes back with full memory. Idempotent — no-op if Emma is alive.
  const respawn = useTauriMutation<void, { sessionId: string }>(
    "respawn_session",
  );
  const [respawnError, setRespawnError] = useState<AppError | null>(null);
  useEffect(() => {
    if (!open) return;
    setRespawnError(null);
    respawn.mutate(
      { sessionId: EMMA_SESSION_ID },
      { onError: (err) => setRespawnError(err) },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const { data: initial = [] } = useTauriQuery<AgentMessage[]>(
    "get_session_messages",
    { sessionId: EMMA_SESSION_ID, sinceId: null },
    { enabled: open },
  );

  const setMessages = useChatStore((s) => s.setMessages);
  const applyBatch = useChatStore((s) => s.applyBatch);
  const messages = useChatStore(
    (s) => s.messages[EMMA_SESSION_ID] ?? EMPTY_MESSAGES,
  );

  useEffect(() => {
    if (open && initial.length > 0) {
      setMessages(EMMA_SESSION_ID, initial);
    }
  }, [open, initial, setMessages]);

  useTauriEvent<AgentMessage[]>(
    "agent:messages:batch",
    (batch) => {
      const forEmma = batch.filter((m) => m.session_id === EMMA_SESSION_ID);
      if (forEmma.length > 0) applyBatch(forEmma);
    },
    [applyBatch],
  );

  // Live status derivation for the status dot. Reuses existing data:
  //   - awaiting: Emma has a parked choice
  //   - thinking: last message is the user's (Emma hasn't replied yet)
  //   - idle: last message is Emma's (or there are no messages)
  const { data: pending = [] } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
    {},
    { enabled: open },
  );
  const status = useMemo<"idle" | "thinking" | "awaiting">(() => {
    if (pending.some((p) => p.session_id === EMMA_SESSION_ID)) return "awaiting";
    if (messages.length === 0) return "idle";
    return messages[messages.length - 1].author !== "emma" ? "thinking" : "idle";
  }, [pending, messages]);

  const { ref: scrollRef, stuck, scrollToBottom } = useStickyScroll<HTMLDivElement>(
    [messages.length, open],
  );

  // Escape-to-close, scoped to overlay-open so it doesn't fight other Escape
  // handlers when Emma isn't visible.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, setOpen]);

  // Input state lives at the overlay level so a stale draft survives
  // accidental focus loss (the terminal feels broken when typed text
  // vanishes on re-render). Cleared by the send handler on success.
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const handleSend = async (e: FormEvent) => {
    e.preventDefault();
    const text = input.trim();
    if (!text || sending) return;
    setSending(true);
    try {
      await invoke("broadcast_message", {
        sessionId: EMMA_SESSION_ID,
        text,
      });
      setInput("");
    } finally {
      setSending(false);
    }
  };

  if (!open) return null;

  return (
    <aside
      role="dialog"
      aria-label="Emma terminal"
      className={cn(
        "fixed inset-y-0 right-0 z-30 flex w-full flex-col",
        "border-l border-outline-variant bg-background shadow-2xl md:w-1/2",
      )}
    >
      <header className="flex h-12 flex-shrink-0 items-center justify-between border-b border-outline-variant bg-surface-container px-4">
        <div className="flex items-center gap-2">
          <TerminalIcon className="text-primary" />
          <h2 className="font-headline-md text-headline-md text-on-surface">
            Emma Terminal
          </h2>
          <span
            aria-hidden
            className="ml-1 size-2 rounded-full bg-primary"
            title="Emma — brand"
          />
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5">
            <span
              aria-hidden
              className={cn(
                "size-1.5 rounded-full",
                status === "awaiting" && "bg-red-400",
                status === "thinking" && "animate-pulse bg-amber-400",
                status === "idle" && "bg-emerald-400",
              )}
            />
            <span className="font-code-sm text-code-sm text-on-surface-variant">
              {status}
            </span>
          </div>
          <button
            type="button"
            onClick={() => setOpen(false)}
            aria-label="Close Emma"
            className="rounded p-1 text-on-surface-variant transition-colors hover:text-on-surface"
          >
            <CloseIcon />
          </button>
        </div>
      </header>

      {respawnError && (
        <div className="flex-shrink-0 border-b border-outline-variant bg-error-container/30 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
          <span className="font-semibold">Emma spawn failed:</span>{" "}
          {respawnError.message}{" "}
          <button
            className="ml-2 underline hover:text-error"
            onClick={() => {
              setRespawnError(null);
              respawn.mutate(
                { sessionId: EMMA_SESSION_ID },
                { onError: (err) => setRespawnError(err) },
              );
            }}
          >
            retry
          </button>
        </div>
      )}
      <div
        ref={scrollRef}
        className="relative flex-1 overflow-auto px-4 py-4"
      >
        {messages.length === 0 ? (
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            <span className="text-primary">$</span> initializing terminal session…
          </p>
        ) : (
          <div className="space-y-4">
            {messages.map((m, i) => {
              const groupedWithPrev =
                i > 0 &&
                m.kind !== "phase_change" &&
                messages[i - 1].kind !== "phase_change" &&
                messages[i - 1].author === m.author;
              return (
                <EmmaTerminalMessage
                  key={m.id}
                  message={m}
                  groupedWithPrev={groupedWithPrev}
                />
              );
            })}
          </div>
        )}
        {!stuck && messages.length > 0 && (
          <div className="pointer-events-none sticky bottom-0 flex justify-end pr-1 pt-2">
            <button
              onClick={scrollToBottom}
              className={cn(
                "pointer-events-auto inline-flex items-center gap-1 rounded",
                "border border-outline-variant bg-surface-container px-3 py-1",
                "font-code-sm text-code-sm text-on-surface shadow-lg",
                "transition-colors hover:border-primary hover:text-primary",
              )}
            >
              ↓ Jump to latest
            </button>
          </div>
        )}
      </div>

      <div className="flex-shrink-0 border-t border-outline-variant bg-surface-container p-4">
        <form onSubmit={handleSend} className="relative flex items-center">
          <span
            aria-hidden
            className="pointer-events-none absolute left-3 font-code-sm text-code-sm text-primary"
          >
            $
          </span>
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Type command…"
            disabled={sending}
            className={cn(
              "w-full rounded border border-outline-variant bg-surface-container-lowest",
              "py-2 pl-8 pr-10 font-code-sm text-code-sm text-on-surface caret-primary",
              "placeholder:text-on-surface-variant",
              "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
              "disabled:opacity-50",
            )}
          />
          <button
            type="submit"
            disabled={!input.trim() || sending}
            aria-label="Send"
            className="absolute right-2 rounded p-1 text-primary transition-colors hover:text-primary-fixed disabled:opacity-40"
          >
            <SendIcon />
          </button>
        </form>
      </div>
    </aside>
  );
}

// ============================================================================
// Terminal-block message — inline replacement for the shared <ChatMessage>.
// Kept Emma-local so SessionView's bubble-style messages aren't affected.
// ============================================================================

interface EmmaTerminalMessageProps {
  message: AgentMessage;
  groupedWithPrev: boolean;
}

const AUTHOR_TEXT: Record<string, string> = {
  emma: "text-primary",
  system: "text-tertiary",
  user: "text-tertiary",
};

const AUTHOR_DOT: Record<string, string> = {
  emma: "bg-primary",
  system: "bg-tertiary",
  user: "bg-tertiary",
};

function EmmaTerminalMessage({ message, groupedWithPrev }: EmmaTerminalMessageProps) {
  if (message.kind === "phase_change") {
    return (
      <div className="my-2 text-center font-code-sm text-code-sm italic text-on-surface-variant">
        — {message.content} —
      </div>
    );
  }

  const authorText = AUTHOR_TEXT[message.author] ?? "text-on-surface-variant";
  const authorDot = AUTHOR_DOT[message.author] ?? "bg-on-surface-variant";

  return (
    <article className="flex flex-col gap-2">
      {!groupedWithPrev && (
        <header className="flex items-center gap-2">
          <span
            aria-hidden
            className={cn("size-2 rounded-full", authorDot)}
          />
          <span
            className={cn(
              "font-label-caps text-label-caps uppercase",
              authorText,
            )}
          >
            {message.author}
          </span>
          <span className="font-code-sm text-code-sm text-on-surface-variant">
            {formatClockTime(message.created_at)}
          </span>
        </header>
      )}
      <div className="rounded-lg border border-outline-variant/30 bg-surface-container-high p-3">
        <p className="whitespace-pre-wrap font-code-sm text-code-sm text-on-surface">
          {message.content}
        </p>
      </div>
    </article>
  );
}

// ============================================================================
// Icons (inline SVG — no Material Symbols dep)
// ============================================================================

function TerminalIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="4 17 10 11 4 5" />
      <line x1="12" y1="19" x2="20" y2="19" />
    </svg>
  );
}

function SendIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-[18px]", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="22" y1="2" x2="11" y2="13" />
      <polygon points="22 2 15 22 11 13 2 9 22 2" />
    </svg>
  );
}

function CloseIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-4", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  );
}
