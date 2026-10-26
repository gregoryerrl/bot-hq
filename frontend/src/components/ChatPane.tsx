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
import { ChatMessage, MessageHeader } from "./ChatMessage";
import { ToolBurst } from "./ToolBurst";
import { cn } from "../lib/cn";
import { authorLabel, useParticipantLabels } from "../lib/participants";
import { chatRows } from "../lib/chatRows";
import type { AgentMessage } from "../lib/bindings";
import { SkipIcon } from "./icons";
import { Button } from "./ui/Button";

// Stable reference so the zustand selector doesn't return a fresh array per
// call (would trigger infinite re-renders via Object.is).
const EMPTY_MESSAGES: AgentMessage[] = [];
/** Rows per history page — the mount read and each "load older" step. */
export const CHAT_PAGE = 300;

/** How close to the bottom (px) still counts as "at the bottom" for
 * auto-follow — same threshold the old useStickyScroll hook used. */
const STICKY_THRESHOLD_PX = 80;
/** Rows bot-hq writes itself; they never continue an author's run. */
const HOST_ROWS = new Set(["phase_change", "system_notice", "system_prompt"]);
/** How long after a burst is opened or closed its resize counts as the
 *  user's own (it is measured within a frame or two). */
const TOGGLE_SETTLE_MS = 1000;

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
 * consecutive messages, and tool-burst expansion — expand state lives HERE
 * (keyed by the burst's first row id, which a growing burst keeps) because
 * virtualized rows unmount when scrolled away, which would reset local state.
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

  // The burst the user just opened or closed, and when: its own growth must
  // not scroll the chat (see the virtualizer's size-change rule below).
  const toggled = useRef<{ key: number; at: number } | null>(null);
  // Lifted tool-burst expand state (see doc comment).
  const [expandedIds, setExpandedIds] = useState<Set<number>>(() => new Set());
  const onToggleExpand = useCallback((id: number) => {
    // Opening a burst is the user reading it: the chat stops following the
    // bottom, so the burst's line stays where it was clicked and its calls
    // open below it. Pinned, the growth scrolled the opened list up and out
    // of view (seen in the rig).
    setStuck(false);
    toggled.current = { key: id, at: Date.now() };
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  // Passes compacted (feedback #13): `pass_turn` call/result rows hidden and a
  // run of pass lines folded into one "passed — A · B" row. Tool rows folded
  // into one burst per participant's run (the visual pass, S3). The
  // virtualizer counts the ROWS it renders, not the raw messages.
  const rows = useMemo(() => chatRows(messages), [messages]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    // Rough single-message height; measureElement corrects per row.
    estimateSize: () => 64,
    overscan: 8,
    getItemKey: (i) => rows[i].key,
  });
  // The virtualizer keeps the view steady when a row ABOVE it changes size by
  // scrolling the same amount. A burst the user just clicked sits at the top
  // edge, which counted as above: opening it scrolled its own line out of
  // view. Its growth now runs downward from where it was clicked.
  // Only for a moment after a click; the library's own rule (which also spares
  // re-measurements while scrolling up) applies the rest of the time.
  const fresh = toggled.current !== null && Date.now() - toggled.current.at < TOGGLE_SETTLE_MS;
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = fresh
    ? (item, _delta, instance) => item.key !== toggled.current?.key && item.start < (instance.scrollOffset ?? 0)
    : undefined;
  const totalSize = virtualizer.getTotalSize();

  // Sticky-bottom tracking (inlined from the retired useStickyScroll hook —
  // the pin effect below additionally needs `totalSize`, which only exists
  // after the virtualizer is constructed on this component's own ref).
  const [stuck, setStuck] = useState(true);
  // The position our own pin last set. Its scroll event arrives after rows
  // that measured taller in between have moved the bottom away; read as the
  // user's scroll, it unstuck a chat that had just opened (seen in the visual
  // pass's rig with a page of tool bursts). Only a scroll AWAY from the pinned
  // position is the user's.
  const pinnedTop = useRef<number | null>(null);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => {
      if (pinnedTop.current !== null && Math.abs(el.scrollTop - pinnedTop.current) < 1) return;
      pinnedTop.current = null;
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
    pinnedTop.current = el.scrollTop;
  }, [messages.length, totalSize, stuck]);

  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    pinnedTop.current = el.scrollTop;
    setStuck(true);
  }, []);

  const items = virtualizer.getVirtualItems();


  return (
    <div
      ref={scrollRef}
      className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-3"
    >
      {messagesLoading && messages.length === 0 ? (
        <MessagesSkeleton />
      ) : messages.length === 0 ? (
        <p className="font-body-md text-body-md text-muted">
          No messages yet…
        </p>
      ) : (
        <>
        {mayHaveOlder && (
          <div className="mb-2 flex justify-center">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void loadOlder()}
              disabled={loadingOlder}
            >
              {loadingOlder ? "Loading older…" : `Load ${CHAT_PAGE} older`}
            </Button>
          </div>
        )}
        <div
          className="relative w-full"
          style={{ height: `${totalSize}px` }}
        >
          {items.map((vi) => {
            const row = rows[vi.index];
            const prevRow = vi.index > 0 ? rows[vi.index - 1] : null;
            if (row.kind === "passes") {
              return (
                <div
                  key={vi.key}
                  data-index={vi.index}
                  ref={virtualizer.measureElement}
                  className="absolute left-0 top-0 w-full"
                  style={{ transform: `translateY(${vi.start}px)` }}
                >
                  <PassLine authors={row.authors} labels={labels} />
                </div>
              );
            }
            // The byline shows once per run of one author's rows: a burst
            // after that author's prose (or prose after its burst) reads as
            // the same turn. A phase change or a host notice always breaks the
            // run — a notice is stored with the synthetic author "user", and
            // the user's next message must still get its own byline.
            const prevAuthor =
              prevRow === null
                ? null
                : prevRow.kind === "burst"
                  ? prevRow.author
                  : prevRow.kind === "message" && !HOST_ROWS.has(prevRow.message.kind)
                    ? prevRow.message.author
                    : null;
            if (row.kind === "burst") {
              const grouped = prevAuthor === row.author;
              return (
                <div
                  key={vi.key}
                  data-index={vi.index}
                  ref={virtualizer.measureElement}
                  className="absolute left-0 top-0 w-full"
                  style={{ transform: `translateY(${vi.start}px)` }}
                >
                  <article className={grouped ? "mt-0" : "mt-3"}>
                    {!grouped && (
                      <MessageHeader label={authorLabel(row.author, labels)} createdAt={row.firstAt} hues={hues} />
                    )}
                    <ToolBurst
                      burst={row}
                      expanded={expandedIds.has(row.key)}
                      onToggle={() => onToggleExpand(row.key)}
                      onViewFile={onViewFile}
                    />
                  </article>
                </div>
              );
            }
            const m = row.message;
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
                  groupedWithPrev={!HOST_ROWS.has(m.kind) && prevAuthor === m.author}
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
            type="button"
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

/**
 * A run of passes as one muted line (feedback #13): "⏭ passed — HANDS · EYES".
 * The same participant passing several times in a row reads "HANDS ×3".
 */
function PassLine({
  authors,
  labels,
}: {
  authors: string[];
  labels: Parameters<typeof authorLabel>[1];
}) {
  const runs: { author: string; count: number }[] = [];
  for (const a of authors) {
    const last = runs[runs.length - 1];
    if (last && last.author === a) last.count += 1;
    else runs.push({ author: a, count: 1 });
  }
  const text = runs
    .map((r) => `${authorLabel(r.author, labels)}${r.count > 1 ? ` ×${r.count}` : ""}`)
    .join(" · ");
  return (
    <div
      className="my-1.5 break-words text-center font-code-sm text-code-sm text-on-surface-variant"
      data-testid="pass-line"
    >
      <SkipIcon className="mr-1 inline-block size-3 align-middle" />
      passed — {text}
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
