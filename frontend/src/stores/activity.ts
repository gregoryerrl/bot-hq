import { create } from "zustand";

/** Session-level duo activity from the backend `session:activity` event
 *  (mirrors Rust `SessionActivity::as_str`). Drives the chat-input lock +
 *  Stop button (interrupt redesign, Batch 4). */
export type SessionActivity = "idle" | "busy" | "awaiting_user" | "cancelling";

/** Per-agent busy flags. The session-level `SessionActivity` collapses these to
 *  a single `busy`; the chat-input turn-status line needs them split so it can
 *  label WHICH agent is working — and a broadcast sets BOTH busy at once. */
export interface DuoBusy {
  brian: boolean;
  rain: boolean;
}

const NO_BUSY: DuoBusy = { brian: false, rain: false };

interface ActivityStore {
  /** session_id -> current activity. Populated live from `session:activity`
   *  (fires only on change). A session with no entry is treated as idle.
   *  In-memory only — resets on app restart. */
  bySession: Record<string, SessionActivity>;
  /** session_id -> per-agent busy flags, carried alongside the collapsed
   *  `bySession` activity. A missing entry reads as neither-busy. */
  busyBySession: Record<string, DuoBusy>;
  setActivity: (
    sessionId: string,
    activity: SessionActivity,
    busy?: DuoBusy,
  ) => void;
  clearSession: (sessionId: string) => void;
}

export const useActivityStore = create<ActivityStore>((set) => ({
  bySession: {},
  busyBySession: {},
  setActivity: (sessionId, activity, busy = NO_BUSY) =>
    set((s) => ({
      bySession: { ...s.bySession, [sessionId]: activity },
      busyBySession: { ...s.busyBySession, [sessionId]: busy },
    })),
  clearSession: (sessionId) =>
    set((s) => {
      if (!s.bySession[sessionId] && !s.busyBySession[sessionId]) return s;
      const bySession = { ...s.bySession };
      const busyBySession = { ...s.busyBySession };
      delete bySession[sessionId];
      delete busyBySession[sessionId];
      return { bySession, busyBySession };
    }),
}));

/** Should the chat input lock? `busy`/`cancelling` lock it (the duo is
 *  working); `idle` and `awaiting_user` (the user's turn) leave it open.
 *  Undefined = no event yet = assume idle (input open). */
export function isLocked(activity: SessionActivity | undefined): boolean {
  return activity === "busy" || activity === "cancelling";
}
