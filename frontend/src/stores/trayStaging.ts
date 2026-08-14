import { create } from "zustand";

/**
 * **Staged tray picks — answers that travel with Send (rc3 D34).**
 *
 * The user's design, verbatim: *"remove the send button on tray items. On
 * Halt, sending a message will also send all of the answers on all tray
 * items."* While the box is open (nobody working — the same `isLocked` rule
 * the composer uses), clicking a tray option STAGES the pick here instead of
 * resolving it; Send delivers the typed message plus every staged answer as
 * one user response (`send_user_response`), and exactly one ring release
 * fires.
 *
 * What this replaces: each pick resolved on click, and the first one released
 * the ring — so answering the first of three questions while halted started
 * the session and locked the box (D33) before the other two could be answered
 * or a message added.
 *
 * While the box is LOCKED (participants working), picks still resolve
 * immediately — a parked question is answerable any time — but the backend no
 * longer interrupts anyone for it (D34): the answer rides the next turn
 * boundary.
 *
 * In-memory only, like a half-typed draft: a staged pick the user never sends
 * simply leaves the row pending in the tray, which is exactly where it was.
 */
interface TrayStagingStore {
  /** session_id -> choice_id -> picked option. */
  staged: Record<string, Record<string, string>>;
  stage: (sessionId: string, choiceId: string, picked: string) => void;
  unstage: (sessionId: string, choiceId: string) => void;
  /** Drop a whole session's staging — called after a successful Send. */
  clear: (sessionId: string) => void;
}

const NO_PICKS: Record<string, string> = {};

export const useTrayStaging = create<TrayStagingStore>((set) => ({
  staged: {},
  stage: (sessionId, choiceId, picked) =>
    set((s) => ({
      staged: {
        ...s.staged,
        [sessionId]: { ...(s.staged[sessionId] ?? {}), [choiceId]: picked },
      },
    })),
  unstage: (sessionId, choiceId) =>
    set((s) => {
      const forSession = s.staged[sessionId];
      if (!forSession || !(choiceId in forSession)) return s;
      const next = { ...forSession };
      delete next[choiceId];
      return { staged: { ...s.staged, [sessionId]: next } };
    }),
  clear: (sessionId) =>
    set((s) => {
      if (!s.staged[sessionId]) return s;
      const staged = { ...s.staged };
      delete staged[sessionId];
      return { staged };
    }),
}));

/** The staged picks for one session; a stable empty object when none. */
export function stagedFor(
  staged: Record<string, Record<string, string>>,
  sessionId: string,
): Record<string, string> {
  return staged[sessionId] ?? NO_PICKS;
}
