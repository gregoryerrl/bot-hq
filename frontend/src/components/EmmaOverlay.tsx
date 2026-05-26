import { useEffect, useMemo, useState } from "react";
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
import { Button } from "./ui/Button";
import { ChatInput } from "./ChatInput";
import { ChatMessage } from "./ChatMessage";
import { cn } from "../lib/cn";

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
  const [screenshotPending, setScreenshotPending] = useState(false);
  const [screenshotError, setScreenshotError] = useState<string | null>(null);
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

  // Live status for the header dot. Derived from existing data so no new
  // command is needed:
  //   - awaiting: Emma has a parked choice in `list_pending_choices`
  //   - thinking: last message is the user's (Emma hasn't replied yet)
  //   - idle: last message is Emma's (or there are no messages)
  const { data: pending = [] } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
    {},
    { refetchInterval: 3_000, enabled: open },
  );
  const status = useMemo<"idle" | "thinking" | "awaiting">(() => {
    if (pending.some((p) => p.session_id === EMMA_SESSION_ID)) return "awaiting";
    if (messages.length === 0) return "idle";
    return messages[messages.length - 1].author !== "emma" ? "thinking" : "idle";
  }, [pending, messages]);

  const { ref: scrollRef, stuck, scrollToBottom } = useStickyScroll<HTMLDivElement>(
    [messages.length, open],
  );

  // Escape-to-close. Scoped to overlay-open so the shortcut doesn't fight
  // with other Escape handlers when Emma isn't visible. Uses `keydown` on
  // the window so it fires regardless of which child has focus (the chat
  // textarea, the message list, etc.).
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

  if (!open) return null;

  return (
    <>
      {/* Dim scrim behind the overlay. Click anywhere outside Emma to close. */}
      <div
        className="fixed inset-0 z-20 bg-black/40 transition-opacity duration-150"
        onClick={() => setOpen(false)}
        aria-hidden
      />
      <aside
        role="dialog"
        aria-label="Emma chat"
        className="fixed inset-y-0 right-0 z-30 flex w-[min(360px,90vw)] md:w-[420px] xl:w-[480px] flex-col border-l border-default bg-canvas shadow-2xl"
      >
      <header className="flex items-center justify-between border-b border-default px-3 py-2">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold text-neutral-100">Emma</h2>
          <span
            aria-hidden
            className={cn(
              "ml-1 size-1.5 rounded-full",
              status === "awaiting" && "bg-red-400",
              status === "thinking" && "animate-pulse bg-amber-400",
              status === "idle" && "bg-emerald-400",
            )}
          />
          <span className="text-[0.65rem] text-neutral-500">
            {status === "awaiting"
              ? "awaiting"
              : status === "thinking"
                ? "thinking"
                : "idle"}
          </span>
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            title="Capture the bot-hq window and share with Emma"
            disabled={screenshotPending}
            onClick={async () => {
              try {
                setScreenshotPending(true);
                setScreenshotError(null);
                await invoke("capture_window_screenshot", {
                  sessionId: EMMA_SESSION_ID,
                });
              } catch (e) {
                const msg =
                  e && typeof e === "object" && "message" in e
                    ? String((e as { message: unknown }).message)
                    : String(e);
                setScreenshotError(msg);
              } finally {
                setScreenshotPending(false);
              }
            }}
          >
            {screenshotPending ? "…" : "📸"}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setOpen(false)}
            aria-label="Close Emma"
          >
            ×
          </Button>
        </div>
      </header>
      {respawnError && (
        <div className="border-b border-default bg-red-950/30 px-3 py-2 text-xs text-red-200">
          <span className="font-semibold">Emma spawn failed:</span>{" "}
          {respawnError.message}{" "}
          <button
            className="ml-2 underline"
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
      {screenshotError && (
        <div className="border-b border-default bg-red-950/30 px-3 py-2 text-xs text-red-200">
          <span className="font-semibold">Screenshot failed:</span>{" "}
          {screenshotError}
          <button
            className="ml-2 underline"
            onClick={() => setScreenshotError(null)}
          >
            dismiss
          </button>
        </div>
      )}
      <div className="relative flex-1 overflow-hidden">
        <div ref={scrollRef} className="h-full overflow-auto px-3 py-3">
          {messages.length === 0 ? (
            <p className="text-sm text-neutral-500">Say hi to Emma…</p>
          ) : (
            messages.map((m, i) => (
              <ChatMessage
                key={m.id}
                message={m}
                groupedWithPrev={
                  i > 0 &&
                  m.kind !== "phase_change" &&
                  messages[i - 1].kind !== "phase_change" &&
                  messages[i - 1].author === m.author
                }
              />
            ))
          )}
        </div>
        {!stuck && messages.length > 0 && (
          <button
            onClick={scrollToBottom}
            className={cn(
              "absolute bottom-3 right-3 inline-flex items-center gap-1 rounded-full",
              "border border-default bg-overlay px-3 py-1 text-xs text-neutral-200 shadow-lg",
              "hover:border-author-emma hover:text-white transition-colors",
            )}
          >
            ↓
          </button>
        )}
      </div>
      <div className="border-t border-default">
        <ChatInput
          placeholder="Message Emma…"
          onSend={async (text) => {
            await invoke("broadcast_message", {
              sessionId: EMMA_SESSION_ID,
              text,
            });
          }}
        />
      </div>
      </aside>
    </>
  );
}
