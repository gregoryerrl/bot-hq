import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useStickyScroll } from "../hooks/useStickyScroll";
import { useEmmaStore } from "../stores/emma";
import { useChatStore } from "../stores/chat";
import type { AgentMessage } from "../lib/bindings";
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

  const { ref: scrollRef, stuck, scrollToBottom } = useStickyScroll<HTMLDivElement>(
    [messages.length, open],
  );

  if (!open) return null;

  return (
    <aside className="fixed inset-y-0 right-0 z-30 flex w-[420px] flex-col border-l border-default bg-canvas shadow-2xl">
      <header className="flex items-center justify-between border-b border-default px-3 py-2">
        <div className="flex items-center gap-2">
          <span className="size-1.5 rounded-full bg-author-emma" />
          <h2 className="text-sm font-semibold text-neutral-100">Emma</h2>
          <span className="text-[0.65rem] text-neutral-500">chat helper</span>
        </div>
        <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
          ×
        </Button>
      </header>
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
  );
}
