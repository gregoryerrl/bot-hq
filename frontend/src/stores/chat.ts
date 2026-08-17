import { create } from "zustand";
import type { AgentMessage } from "../lib/bindings";

interface ChatState {
  /** session_id → last seen message id. Used to dedupe batched events. */
  watermarks: Record<string, number>;
  /** session_id → ordered messages. */
  messages: Record<string, AgentMessage[]>;
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

export const useChatStore = create<ChatState>((set) => ({
  watermarks: {},
  messages: {},
  setMessages: (sessionId, msgs) =>
    set((s) => ({
      messages: { ...s.messages, [sessionId]: msgs },
      watermarks: {
        ...s.watermarks,
        [sessionId]: msgs.length > 0 ? msgs[msgs.length - 1].id : 0,
      },
    })),
  applyBatch: (msgs) =>
    set((s) => {
      if (msgs.length === 0) return s;
      const messages = { ...s.messages };
      const watermarks = { ...s.watermarks };
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
      }
      return { messages, watermarks };
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
      return { messages: { ...s.messages, [sessionId]: older.concat(current) } };
    }),
  clear: (sessionId) =>
    set((s) => {
      const { [sessionId]: _, ...messages } = s.messages;
      const { [sessionId]: __, ...watermarks } = s.watermarks;
      return { messages, watermarks };
    }),
}));
