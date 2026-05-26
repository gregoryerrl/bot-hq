import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useChatStore } from "../stores/chat";
import { ChatInput } from "../components/ChatInput";
import { DocumentPane } from "../components/DocumentPane";
import { authorColorClass } from "../components/AuthorBadge";
import { cn } from "../lib/cn";
import type {
  AgentMessage,
  AppError,
  PendingChoiceView,
  SessionInfo,
} from "../lib/bindings";
import { Button } from "../components/ui/Button";
import { invoke } from "@tauri-apps/api/core";

// Stable reference — see EmmaOverlay for the same reason.
const EMPTY_MESSAGES: AgentMessage[] = [];

export function SessionView() {
  const { sessionId = "" } = useParams<{ sessionId: string }>();

  const { data: session, error: sessionError } = useTauriQuery<SessionInfo | null>(
    "get_session",
    { sessionId },
  );

  // Respawn agents on mount. Idempotent — `ensure_session_started` returns
  // immediately if Brian/Rain are already running. Mirrors the Slint-era
  // click-to-respawn flow; reads `brian_claude_session_id` /
  // `rain_claude_session_id` and passes `--resume <uuid>` so the agents
  // come back with full memory.
  const respawn = useTauriMutation<void, { sessionId: string }>(
    "respawn_session",
  );
  const [respawnError, setRespawnError] = useState<AppError | null>(null);
  useEffect(() => {
    if (!sessionId) return;
    setRespawnError(null);
    respawn.mutate(
      { sessionId },
      { onError: (err) => setRespawnError(err) },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  const { data: initialMsgs = [] } = useTauriQuery<AgentMessage[]>(
    "get_session_messages",
    { sessionId, sinceId: null },
    { enabled: !!sessionId },
  );

  const messages = useChatStore((s) => s.messages[sessionId] ?? EMPTY_MESSAGES);
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
        {sessionError ? (
          <>
            <p className="mb-2 text-red-300">
              Failed to load session: {sessionError.message}
            </p>
            <p className="text-xs text-neutral-500">id: {sessionId}</p>
          </>
        ) : (
          <>Session not found.</>
        )}{" "}
        <Link to="/" className="text-blue-400 underline">
          Back to dashboard
        </Link>
      </div>
    );
  }

  return (
    // `grid-rows-1` makes the single implicit row take the full container
    // height instead of shrinking to content. Without it the inner section's
    // `h-full` has no defined parent height, the messages div grows past the
    // viewport, and the ChatInput at the bottom becomes unreachable.
    <div className="grid h-full grid-cols-[3fr_2fr] grid-rows-1">
      <section className="flex h-full min-h-0 flex-col border-r border-neutral-800">
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

        {respawnError && (
          <div className="border-b border-neutral-800 bg-red-950/30 px-4 py-2 text-xs text-red-200">
            <span className="font-semibold">Agent spawn failed:</span>{" "}
            {respawnError.message}{" "}
            <button
              className="ml-2 underline"
              onClick={() => {
                setRespawnError(null);
                respawn.mutate(
                  { sessionId },
                  { onError: (err) => setRespawnError(err) },
                );
              }}
            >
              retry
            </button>
          </div>
        )}

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
                      choiceId: choicesForSession[0].choice_id,
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

        <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
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
            onSend={async (text) => {
              await invoke("broadcast_message", { sessionId, text });
            }}
          />
        </div>
      </section>

      <DocumentPane sessionId={sessionId} />
    </div>
  );
}
