import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useEmmaStore } from "../stores/emma";
import { useChatStore } from "../stores/chat";
import type { AgentMessage } from "../lib/bindings";
import { Button } from "./ui/Button";
import { ChatInput } from "./ChatInput";
import { authorColorClass } from "./AuthorBadge";
import { cn } from "../lib/cn";

const EMMA_SESSION_ID = "emma";
// Stable reference for the "no messages yet" branch — a fresh `[]` literal
// inside a zustand selector triggers infinite re-renders because
// `Object.is([], [])` is false, so the selector output looks like it changed
// on every render.
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
  const messages = useChatStore((s) => s.messages[EMMA_SESSION_ID] ?? EMPTY_MESSAGES);

  useEffect(() => {
    if (open && initial.length > 0) {
      setMessages(EMMA_SESSION_ID, initial);
    }
  }, [open, initial, setMessages]);

  useTauriEvent<AgentMessage[]>(
    "agent.messages.batch",
    (batch) => {
      const forEmma = batch.filter((m) => m.session_id === EMMA_SESSION_ID);
      if (forEmma.length > 0) applyBatch(forEmma);
    },
    [applyBatch],
  );

  if (!open) return null;

  return (
    <aside className="fixed inset-y-0 right-0 z-30 flex w-[420px] flex-col border-l border-neutral-800 bg-neutral-950 shadow-2xl">
      <header className="flex items-center justify-between border-b border-neutral-800 px-3 py-2">
        <div className="flex items-center gap-2">
          <span className="size-1.5 rounded-full bg-author-emma" />
          <h2 className="text-sm font-semibold text-neutral-100">Emma</h2>
        </div>
        <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
          ×
        </Button>
      </header>
      <div className="flex-1 overflow-auto px-3 py-3">
        {messages.length === 0 ? (
          <p className="text-sm text-neutral-500">Say hi to Emma…</p>
        ) : (
          messages.map((m) => (
            <div key={m.id} className="mb-3">
              <div className={cn("text-[0.65rem] uppercase tracking-wide", authorColorClass(m.author))}>
                {m.author}
              </div>
              <div className="whitespace-pre-wrap text-sm text-neutral-200">
                {m.content}
              </div>
            </div>
          ))
        )}
      </div>
      <div className="border-t border-neutral-800">
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
