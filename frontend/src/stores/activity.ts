import { create } from "zustand";

/** Session-level duo activity from the backend `session:activity` event
 *  (mirrors Rust `SessionActivity::as_str`). Drives the chat-input lock +
 *  Stop button (interrupt redesign, Batch 4). */
export type SessionActivity = "idle" | "busy" | "awaiting_user" | "cancelling";

interface ActivityStore {
  /** session_id -> current activity. Populated live from `session:activity`
   *  (fires only on change). A session with no entry is treated as idle.
   *  In-memory only — resets on app restart. */
  bySession: Record<string, SessionActivity>;
  setActivity: (sessionId: string, activity: SessionActivity) => void;
  clearSession: (sessionId: string) => void;
}

export const useActivityStore = create<ActivityStore>((set) => ({
  bySession: {},
  setActivity: (sessionId, activity) =>
    set((s) => ({
      bySession: { ...s.bySession, [sessionId]: activity },
    })),
  clearSession: (sessionId) =>
    set((s) => {
      if (!s.bySession[sessionId]) return s;
      const next = { ...s.bySession };
      delete next[sessionId];
      return { bySession: next };
    }),
}));

/** Should the chat input lock? `busy`/`cancelling` lock it (the duo is
 *  working); `idle` and `awaiting_user` (the user's turn) leave it open.
 *  Undefined = no event yet = assume idle (input open). */
export function isLocked(activity: SessionActivity | undefined): boolean {
  return activity === "busy" || activity === "cancelling";
}
