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
      // Group by session_id; append in order, skipping any that don't
      // advance the watermark (defensive against duplicate events).
      for (const msg of msgs) {
        const current = messages[msg.session_id] ?? [];
        const wm = watermarks[msg.session_id] ?? 0;
        if (msg.id <= wm) continue;
        messages[msg.session_id] = [...current, msg];
        watermarks[msg.session_id] = msg.id;
      }
      return { messages, watermarks };
    }),
  clear: (sessionId) =>
    set((s) => {
      const { [sessionId]: _, ...messages } = s.messages;
      const { [sessionId]: __, ...watermarks } = s.watermarks;
      return { messages, watermarks };
    }),
}));
