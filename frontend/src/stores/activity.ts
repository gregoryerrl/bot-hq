import { create } from "zustand";

/** Session-level duo activity from the backend `session:activity` event
 *  (mirrors Rust `SessionActivity::as_str`). Drives the chat-input lock +
 *  Stop button (interrupt redesign, Batch 4). `paused` = Stop landed: agents
 *  interrupted + all auto-wakes held; the ChatInput shows the paused bar
 *  (Resume / Close) with the textarea open for a steer. */
export type SessionActivity =
  | "idle"
  | "busy"
  | "awaiting_user"
  | "cancelling"
  | "paused";

/** Participant slug → mid-turn. The session-level `SessionActivity` collapses
 *  these to a single `busy`; the chat-input turn-status line needs them split so
 *  it can say WHICH participant is working — and a broadcast sets every one of
 *  them busy at once.
 *
 *  The slug is an internal key: what the line PRINTS comes from the session's
 *  roster (rc3 D10), never from this map. */
export type AgentBusy = Record<string, boolean>;

const NO_BUSY: AgentBusy = {};

interface ActivityStore {
  /** session_id -> current activity. Populated live from `session:activity`
   *  (fires only on change). A session with no entry is treated as idle.
   *  In-memory only — resets on app restart. */
  bySession: Record<string, SessionActivity>;
  /** session_id -> per-agent busy flags, carried alongside the collapsed
   *  `bySession` activity. A missing entry reads as neither-busy. */
  busyBySession: Record<string, AgentBusy>;
  setActivity: (
    sessionId: string,
    activity: SessionActivity,
    busy?: AgentBusy,
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
 *  working); `idle`, `awaiting_user`, and `paused` (the user's turn — steer,
 *  resume, or close) leave it open. Undefined = no event yet = assume idle
 *  (input open). */
export function isLocked(activity: SessionActivity | undefined): boolean {
  return activity === "busy" || activity === "cancelling";
}
