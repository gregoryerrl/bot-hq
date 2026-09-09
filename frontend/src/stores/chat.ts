import { create } from "zustand";
import type { AgentMessage } from "../lib/bindings";

interface ChatState {
  /** session_id → last seen message id. Used to dedupe batched events. */
  watermarks: Record<string, number>;
  /** session_id → ordered messages. */
  messages: Record<string, AgentMessage[]>;
  /** session_id → the `tool_use_id`s that have a `tool_result` row. A
   *  `tool_use` whose id is not in here is STILL RUNNING (the chat renders it
   *  as such). Kept INCREMENTALLY here — every path that admits rows adds to
   *  it — rather than re-derived from the whole list per batch: that memo
   *  walked and `JSON.parse`d every result row on every 50 ms batch, over a
   *  list that grows 300 rows per "load older" (round 9). A new Set identity
   *  is published only when an id was actually added, so consumers re-render
   *  no more than the data changes. */
  resolvedToolIds: Record<string, ReadonlySet<string>>;
  /** Replace the entire message list for a session (initial fetch). */
  setMessages: (sessionId: string, msgs: AgentMessage[]) => void;
  /** Apply a batch of new messages, advancing the watermark. */
  applyBatch: (msgs: AgentMessage[]) => void;
  /** Prepend an OLDER page (chronological, all ids below the current first)
   *  — the "load older" read (round 8). Leaves the watermark alone: the
   *  watermark is the newest id, and older rows never advance it. */
  prependOlder: (sessionId: string, msgs: AgentMessage[]) => void;
  /** Clear a session (close). */
  clear: (sessionId: string) => void;
}

/** The `tool_use_id` a `tool_result` row answers, if the row parses. */
function toolResultId(m: AgentMessage): string | null {
  if (m.kind !== "tool_result") return null;
  try {
    const id = (JSON.parse(m.content) as { tool_use_id?: unknown }).tool_use_id;
    return typeof id === "string" ? id : null;
  } catch {
    // Unparseable result row — the worst case is a call that keeps showing as
    // running, never a finished one showing as done.
    return null;
  }
}

/** `prev` plus the result ids in `rows`; the SAME Set (identity preserved)
 *  when nothing new was found. */
function withResolved(prev: ReadonlySet<string> | undefined, rows: AgentMessage[]): ReadonlySet<string> {
  let next: Set<string> | null = null;
  for (const m of rows) {
    const id = toolResultId(m);
    if (id === null || (prev?.has(id) ?? false) || next?.has(id)) continue;
    next ??= new Set(prev);
    next.add(id);
  }
  return next ?? prev ?? EMPTY_SET;
}

const EMPTY_SET: ReadonlySet<string> = new Set();

export const useChatStore = create<ChatState>((set) => ({
  watermarks: {},
  messages: {},
  resolvedToolIds: {},
  setMessages: (sessionId, msgs) =>
    set((s) => ({
      messages: { ...s.messages, [sessionId]: msgs },
      watermarks: {
        ...s.watermarks,
        [sessionId]: msgs.length > 0 ? msgs[msgs.length - 1].id : 0,
      },
      // A replace re-seeds from scratch — the previous set may name rows the
      // new list no longer holds.
      resolvedToolIds: {
        ...s.resolvedToolIds,
        [sessionId]: withResolved(undefined, msgs),
      },
    })),
  applyBatch: (msgs) =>
    set((s) => {
      if (msgs.length === 0) return s;
      const messages = { ...s.messages };
      const watermarks = { ...s.watermarks };
      const resolvedToolIds = { ...s.resolvedToolIds };
      // Accumulate per-session appends first, then splice each session's array
      // ONCE. Spreading `[...current, msg]` inside the loop was O(N·K): a
      // 20-message batch (BatchEmitter's FLUSH_AT_N) for one session copied the
      // full length-N history up to 20 times. Skip any id that doesn't advance
      // the watermark (defensive against duplicate events).
      const appends: Record<string, AgentMessage[]> = {};
      for (const msg of msgs) {
        const wm = watermarks[msg.session_id] ?? 0;
        if (msg.id <= wm) continue;
        (appends[msg.session_id] ??= []).push(msg);
        watermarks[msg.session_id] = msg.id;
      }
      for (const sessionId of Object.keys(appends)) {
        const current = messages[sessionId] ?? [];
        messages[sessionId] = current.concat(appends[sessionId]);
        resolvedToolIds[sessionId] = withResolved(resolvedToolIds[sessionId], appends[sessionId]);
      }
      return { messages, watermarks, resolvedToolIds };
    }),
  prependOlder: (sessionId, msgs) =>
    set((s) => {
      if (msgs.length === 0) return s;
      const current = s.messages[sessionId] ?? [];
      const firstId = current.length > 0 ? current[0].id : Number.MAX_SAFE_INTEGER;
      // Only rows genuinely older than what is held; a page that overlaps
      // (a double click, a race with the mount read) must not duplicate.
      const older = msgs.filter((m) => m.id < firstId);
      if (older.length === 0) return s;
      return {
        messages: { ...s.messages, [sessionId]: older.concat(current) },
        resolvedToolIds: {
          ...s.resolvedToolIds,
          [sessionId]: withResolved(s.resolvedToolIds[sessionId], older),
        },
      };
    }),
  clear: (sessionId) =>
    set((s) => {
      const { [sessionId]: _, ...messages } = s.messages;
      const { [sessionId]: __, ...watermarks } = s.watermarks;
      const { [sessionId]: ___, ...resolvedToolIds } = s.resolvedToolIds;
      return { messages, watermarks, resolvedToolIds };
    }),
}));
