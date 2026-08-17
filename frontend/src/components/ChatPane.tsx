import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useChatStore } from "../stores/chat";
import { ChatMessage } from "./ChatMessage";
import { cn } from "../lib/cn";
import { authorLabel, useParticipantLabels } from "../lib/participants";
import type { AgentMessage } from "../lib/bindings";

// Stable reference so the zustand selector doesn't return a fresh array per
// call (would trigger infinite re-renders via Object.is).
const EMPTY_MESSAGES: AgentMessage[] = [];
/** Rows per history page — the mount read and each "load older" step. */
export const CHAT_PAGE = 300;

/** How close to the bottom (px) still counts as "at the bottom" for
 * auto-follow — same threshold the old useStickyScroll hook used. */
const STICKY_THRESHOLD_PX = 80;

/**
 * The session chat: message history + live batches, virtualized.
 *
 * Owns the chat-store subscription and the `agent:messages:batch` listener so
 * per-batch re-renders stop at this component — the SessionView shell (header,
 * subtabs, DocumentPane, ChatInput) no longer re-renders on every batch.
 * Rendering goes through `useVirtualizer`, so only the visible window (+
 * overscan) of a session's history exists in the DOM regardless of length.
 *
 * Preserved behaviors: auto-follow when the user is at the bottom, the
 * "↓ Jump to latest" pill when they've scrolled up, author-grouping of
 * consecutive messages, and tool-pill expansion — expand state lives HERE
 * (keyed by message id) because virtualized rows unmount when scrolled away,
 * which would reset ChatMessage-local state.
 */
export function ChatPane({
  sessionId,
  onViewFile,
}: {
  sessionId: string;
  /** Opens a file named by a tool call in the session's file viewer (round 8). */
  onViewFile?: (path: string) => void;
}) {
  // The mount read is the TAIL, not the whole history (round 8, N2): the
  // largest session held 3,412 rows / 6.5 MB, all of it pulled into one Vec
  // and across the IPC boundary on every mount, and the pane only ever shows
  // the bottom. Older pages arrive on demand via "Load older" below; a full
  // page (`length === CHAT_PAGE`) is what says there may be more.
  const { data: initialMsgs = [], isLoading: messagesLoading } = useTauriQuery<
    AgentMessage[]
  >(
    "get_session_messages",
    // `CHAT_PAGE + 1`: the extra row is a PROBE, not a rendered message — its
    // presence is what says older rows exist (`channel_page`'s `limit+1`
    // idiom). Trimmed before the store sees it.
    { sessionId, sinceId: null, limit: CHAT_PAGE + 1 },
    { enabled: !!sessionId },
  );

  // rc3 D10: a message's stored `author` is a participant slug and is never
  // shown. The roster turns it into `ROLE · Model` for the byline.
  const { labels, hues } = useParticipantLabels(sessionId);

  const messages = useChatStore(
    (s) => s.messages[sessionId] ?? EMPTY_MESSAGES,
  );
  const setMessages = useChatStore((s) => s.setMessages);
  const applyBatch = useChatStore((s) => s.applyBatch);

  // "There may be more above": true until a page comes back short. Reset
  // when the mount read lands (a refetch on resync re-seeds the store).
  const [mayHaveOlder, setMayHaveOlder] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  useEffect(() => {
    if (initialMsgs.length > 0) {
      // The probe row (the oldest of CHAT_PAGE + 1) is dropped; a full probe
      // means there IS an older row, not "there might be".
      const more = initialMsgs.length > CHAT_PAGE;
      setMessages(sessionId, more ? initialMsgs.slice(1) : initialMsgs);
      setMayHaveOlder(more);
    }
  }, [initialMsgs, sessionId, setMessages]);
  const prependOlder = useChatStore((s) => s.prependOlder);
  const loadOlder = useCallback(async () => {
    const first = useChatStore.getState().messages[sessionId]?.[0];
    if (!first || loadingOlder) return;
    setLoadingOlder(true);
    try {
      const page = await invoke<AgentMessage[]>("get_session_messages", {
        sessionId,
        sinceId: null,
        beforeId: first.id,
        limit: CHAT_PAGE + 1,
      });
      // Same probe: a page of CHAT_PAGE + 1 means more remain; the probe row
      // is trimmed so exactly CHAT_PAGE rows are prepended.
      const more = page.length > CHAT_PAGE;
      prependOlder(sessionId, more ? page.slice(1) : page);
      setMayHaveOlder(more);
    } catch {
      // Leave the button; the next click retries.
    } finally {
      setLoadingOlder(false);
    }
  }, [sessionId, loadingOlder, prependOlder]);

  useTauriEvent<AgentMessage[]>(
    "agent:messages:batch",
    (batch) => {
      const forSession = batch.filter((m) => m.session_id === sessionId);
      if (forSession.length > 0) applyBatch(forSession);
    },
    [sessionId, applyBatch],
  );

  // Lifted tool-pill expand state (see doc comment).
  const [expandedIds, setExpandedIds] = useState<Set<number>>(() => new Set());
  const onToggleExpand = useCallback((id: number) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => scrollRef.current,
    // Rough single-message height; measureElement corrects per row.
    estimateSize: () => 64,
    overscan: 8,
    getItemKey: (i) => messages[i].id,
  });
  const totalSize = virtualizer.getTotalSize();

  // Sticky-bottom tracking (inlined from the retired useStickyScroll hook —
  // the pin effect below additionally needs `totalSize`, which only exists
  // after the virtualizer is constructed on this component's own ref).
  const [stuck, setStuck] = useState(true);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => {
      const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
      setStuck(distance < STICKY_THRESHOLD_PX);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  // Pin to the bottom while stuck. Re-runs when messages arrive AND when the
  // virtualizer's measured total grows (estimated rows measuring taller would
  // otherwise drift the view off the bottom). Layout effect: reposition
  // before paint so following a live stream doesn't jitter.
  useLayoutEffect(() => {
    if (!stuck) return;
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [messages.length, totalSize, stuck]);

  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    setStuck(true);
  }, []);

  const items = virtualizer.getVirtualItems();

  // Which tool calls have already come back. A `tool_use` whose id isn't in
  // here is STILL RUNNING — without this a five-minute `cargo build --release`
  // and a 20ms `Read` render identically and nothing on screen moves while the
  // build runs. Computed over the whole list (not just the virtual window) so
  // scrolling can't turn a finished call back into a running one, and memoized
  // so it isn't re-parsed on every scroll frame.
  const resolvedToolIds = useMemo(() => {
    const ids = new Set<string>();
    for (const m of messages) {
      if (m.kind !== "tool_result") continue;
      try {
        const id = (JSON.parse(m.content) as { tool_use_id?: unknown })
          .tool_use_id;
        if (typeof id === "string") ids.add(id);
      } catch {
        // Unparseable result row — skip it; the worst case is a call that
        // keeps showing as running, never a finished one showing as done.
      }
    }
    return ids;
  }, [messages]);

  return (
    <div
      ref={scrollRef}
      className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-3"
    >
      {messagesLoading && messages.length === 0 ? (
        <MessagesSkeleton />
      ) : messages.length === 0 ? (
        <p className="font-body-md text-body-md text-on-surface-variant">
          No messages yet…
        </p>
      ) : (
        <>
        {mayHaveOlder && (
          <div className="mb-2 flex justify-center">
            <button
              type="button"
              onClick={() => void loadOlder()}
              disabled={loadingOlder}
              className="rounded border border-outline-variant px-2.5 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:text-on-surface disabled:opacity-60"
            >
              {loadingOlder ? "Loading older…" : `Load ${CHAT_PAGE} older`}
            </button>
          </div>
        )}
        <div
          className="relative w-full"
          style={{ height: `${totalSize}px` }}
        >
          {items.map((vi) => {
            const m = messages[vi.index];
            const prev = vi.index > 0 ? messages[vi.index - 1] : null;
            return (
              <div
                key={vi.key}
                data-index={vi.index}
                ref={virtualizer.measureElement}
                className="absolute left-0 top-0 w-full"
                style={{ transform: `translateY(${vi.start}px)` }}
              >
                <ChatMessage
                  message={m}
                  authorLabel={authorLabel(m.author, labels)}
                  authorHues={hues}
                  groupedWithPrev={
                    prev !== null &&
                    m.kind !== "phase_change" &&
                    prev.kind !== "phase_change" &&
                    prev.author === m.author
                  }
                  expanded={expandedIds.has(m.id)}
                  onToggleExpand={onToggleExpand}
                  resolvedToolIds={resolvedToolIds}
                  onViewFile={onViewFile}
                />
              </div>
            );
          })}
        </div>
        </>
      )}
      {!stuck && messages.length > 0 && (
        <div className="pointer-events-none sticky bottom-0 flex justify-end pr-1 pt-2">
          <button
            onClick={scrollToBottom}
            className={cn(
              "pointer-events-auto inline-flex items-center gap-1 rounded-full",
              "border border-outline-variant bg-surface-container-highest px-3 py-1 font-code-sm text-code-sm text-on-surface shadow-lg",
              "hover:border-primary hover:text-on-surface transition-colors",
            )}
          >
            ↓ Jump to latest
          </button>
        </div>
      )}
    </div>
  );
}

function MessagesSkeleton() {
  return (
    <div className="space-y-4">
      {[0, 1, 2].map((i) => (
        <div key={i} className="space-y-2">
          <div className="h-3 w-12 animate-pulse rounded bg-surface-container-high" />
          <div className="h-3 w-3/4 animate-pulse rounded bg-surface-container-high" />
          <div className="h-3 w-1/2 animate-pulse rounded bg-surface-container-high" />
        </div>
      ))}
    </div>
  );
}
