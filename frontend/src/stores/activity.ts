import { create } from "zustand";

/** Session-level activity from the backend `session:activity` event
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

/** Is any participant mid-turn? The session-level activity collapses the map to
 *  a single `busy` and ranks `awaiting` above it, so this is the only honest
 *  answer to "is somebody working right now". */
export function anyBusy(busy?: AgentBusy): boolean {
  return Object.values(busy ?? {}).some(Boolean);
}

/**
 * **Should a SEND go through now? — the one rule the whole interrupt model
 * rests on (rc3 D33).**
 *
 * > *"users are never allowed to type while agents are working, no halt = no
 * > type (except for pause button which is the real interrupt)"*
 *
 * D33's rule was about messages LANDING mid-turn. Since the Stage toggle
 * (2026-08-15) the box itself stays writable while agents work: `locked`
 * turns the submit slot into **Stage** — the message is queued and delivered
 * at the next turn boundary — and Pause remains the only interrupt. The
 * paragraphs below describe the lock as it was first shipped (a read-only
 * box); the *reason* still stands, the affordance changed.
 *
 * So: **locked ⟺ somebody is working.** Not "the session says busy" — the
 * per-participant map, because `SessionActivity::derive` ranks `awaiting` ABOVE
 * `busy` and a parked question therefore reported `awaiting_user` while two
 * participants ran. That is the reported screenshot: an open textarea, a banner
 * claiming a halt, and a status line underneath correctly naming the
 * participant mid-turn. The box was typeable because the collapsed enum had
 * lost the fact that anyone was busy.
 *
 * `paused` is the deliberate exception and the point of the whole design: the
 * user pressed Pause, which is the ONE interrupt in the product. Agents are
 * stopped by the time it lands, and the busy flags may still be draining — the
 * user gets the box regardless, because taking it is what they pressed the
 * button for.
 *
 * What this buys, and why it is worth a locked box: **no message can arrive
 * mid-turn.** The alternative considered and rejected was buffering — hold the
 * user's text and deliver it at the next turn boundary — which needs a queue, a
 * delivery point, an "it will land later" affordance, and an answer for what
 * happens when the session halts before the boundary. Locking needs none of
 * that, and it makes the cost visible at the moment the user pays it rather
 * than silently deferring their words.
 *
 * The cost, chosen rather than discovered: **arriving to a working session
 * takes one extra click.** You open a session to fire a prompt and leave; if
 * agents are mid-turn you press Pause first. Deliberate, not free.
 */
export function isLocked(
  activity: SessionActivity | undefined,
  busy?: AgentBusy,
): boolean {
  if (activity === "cancelling") return true;
  // Pause landed: the interrupt the user chose. The box is theirs even if the
  // busy flags have not finished draining.
  if (activity === "paused") return false;
  return activity === "busy" || anyBusy(busy);
}
