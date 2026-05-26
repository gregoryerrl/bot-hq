import { useEffect } from "react";
import { Link, useParams } from "react-router-dom";
import { useTauriQuery } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useChatStore } from "../stores/chat";
import { ChatInput } from "../components/ChatInput";
import { DocumentPane } from "../components/DocumentPane";
import { authorColorClass } from "../components/AuthorBadge";
import { cn } from "../lib/cn";
import type {
  AgentMessage,
  PendingChoiceView,
  SessionInfo,
} from "../lib/bindings";
import { Button } from "../components/ui/Button";
import { invoke } from "@tauri-apps/api/core";

export function SessionView() {
  const { sessionId = "" } = useParams<{ sessionId: string }>();

  const { data: session } = useTauriQuery<SessionInfo | null>("get_session", {
    session_id: sessionId,
  });

  const { data: initialMsgs = [] } = useTauriQuery<AgentMessage[]>(
    "get_session_messages",
    { session_id: sessionId, since_id: null },
    { enabled: !!sessionId },
  );

  const messages = useChatStore((s) => s.messages[sessionId] ?? []);
  const setMessages = useChatStore((s) => s.setMessages);
  const applyBatch = useChatStore((s) => s.applyBatch);

  useEffect(() => {
    if (initialMsgs.length > 0) {
      setMessages(sessionId, initialMsgs);
    }
  }, [initialMsgs, sessionId, setMessages]);

  useTauriEvent<AgentMessage[]>(
    "agent.messages.batch",
    (batch) => {
      const forSession = batch.filter((m) => m.session_id === sessionId);
      if (forSession.length > 0) applyBatch(forSession);
    },
    [sessionId, applyBatch],
  );

  const { data: pendingChoices = [] } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
  );
  const choicesForSession = pendingChoices.filter(
    (c) => c.session_id === sessionId,
  );

  if (!session) {
    return (
      <div className="p-6 text-sm text-neutral-500">
        Session not found.{" "}
        <Link to="/" className="text-blue-400 underline">
          Back to dashboard
        </Link>
      </div>
    );
  }

  return (
    <div className="grid h-full grid-cols-[3fr_2fr]">
      <section className="flex h-full flex-col border-r border-neutral-800">
        <header className="flex items-center justify-between border-b border-neutral-800 px-4 py-3">
          <div>
            <h1 className="text-base font-semibold">{session.title}</h1>
            <p className="text-xs text-neutral-500">
              <Link to="/" className="hover:text-neutral-300">
                ← Dashboard
              </Link>
            </p>
          </div>
        </header>

        {choicesForSession.length > 0 && (
          <div className="border-b border-neutral-800 bg-purple-950/30 px-4 py-2 text-xs">
            <span className="font-semibold text-purple-200">
              Awaiting your choice:
            </span>{" "}
            {choicesForSession[0].question}
            <div className="mt-2 flex flex-wrap gap-1">
              {choicesForSession[0].options.map((opt) => (
                <Button
                  key={opt}
                  size="sm"
                  variant="primary"
                  onClick={() =>
                    invoke("resolve_choice", {
                      choice_id: choicesForSession[0].choice_id,
                      picked: opt,
                    })
                  }
                >
                  {opt}
                </Button>
              ))}
            </div>
          </div>
        )}

        <div className="flex-1 overflow-auto px-4 py-3">
          {messages.length === 0 ? (
            <p className="text-sm text-neutral-500">No messages yet…</p>
          ) : (
            messages.map((m) => (
              <article key={m.id} className="mb-3">
                <div
                  className={cn(
                    "text-[0.65rem] uppercase tracking-wide",
                    authorColorClass(m.author),
                  )}
                >
                  {m.author}
                </div>
                <div className="whitespace-pre-wrap text-sm text-neutral-100">
                  {m.content}
                </div>
              </article>
            ))
          )}
        </div>

        <div className="border-t border-neutral-800">
          <ChatInput
            placeholder="Broadcast to Brian + Rain…"
            onSend={async () => {
              // broadcast_to_session deferred until core::broadcast path
              // ships. See Batch 2's deferred-list. Wire here when ready.
            }}
          />
        </div>
      </section>

      <DocumentPane sessionId={sessionId} />
    </div>
  );
}
