import { create } from "zustand";

/** Agent liveness as reported by the backend `session:agent_health` event
 *  (mirrors Rust `AgentHealth::as_str`). */
export type AgentHealth = "running" | "retrying" | "stalled" | "dead";

/**
 * Participant slug → liveness. The key is the slug the backend emits on
 * `session:agent_health`, and it is an INTERNAL key — rc3 D10 keeps it out of
 * every rendered string (see `lib/participants.ts` for what gets shown).
 *
 * A map rather than two named fields because the roster is the session's, not
 * the product's: the header reads it by the slugs its roster actually has.
 */
export type SessionHealth = Record<string, AgentHealth | undefined>;

interface HealthStore {
  /** session_id -> per-agent health. Populated live from the agent_health
   *  event; an agent with no entry is assumed running (events fire only on
   *  transitions). In-memory only — resets on app restart. */
  bySession: Record<string, SessionHealth>;
  /** session_id -> attention state from the idle-unflagged watchdog
   *  ("idle_unflagged"). No entry = clear. Populated live from
   *  `session:attention` (null state deletes) and seeded on mount from
   *  `get_session_runtime`. Drives the "needs direction" chip. */
  attentionBySession: Record<string, string>;
  setHealth: (sessionId: string, agent: string, health: AgentHealth) => void;
  setAttention: (sessionId: string, state: string | null) => void;
  clearSession: (sessionId: string) => void;
}

export const useHealthStore = create<HealthStore>((set) => ({
  bySession: {},
  attentionBySession: {},
  setHealth: (sessionId, agent, health) =>
    set((s) => ({
      bySession: {
        ...s.bySession,
        [sessionId]: { ...s.bySession[sessionId], [agent]: health },
      },
    })),
  setAttention: (sessionId, state) =>
    set((s) => {
      if (state === null) {
        if (!(sessionId in s.attentionBySession)) return s;
        const attentionBySession = { ...s.attentionBySession };
        delete attentionBySession[sessionId];
        return { attentionBySession };
      }
      return {
        attentionBySession: { ...s.attentionBySession, [sessionId]: state },
      };
    }),
  clearSession: (sessionId) =>
    set((s) => {
      const hadAgent = !!s.bySession[sessionId];
      const hadAttention = sessionId in s.attentionBySession;
      if (!hadAgent && !hadAttention) return s;
      const bySession = { ...s.bySession };
      delete bySession[sessionId];
      const attentionBySession = { ...s.attentionBySession };
      delete attentionBySession[sessionId];
      return { bySession, attentionBySession };
    }),
}));

/** Severity order for {@link worstHealth}: worst first. */
const HEALTH_RANK: AgentHealth[] = ["dead", "stalled", "retrying", "running"];

/** Worst-of a session's agents, for a single tile-level dot: dead > stalled >
 *  retrying > running. Returns undefined when there's no health data (assume
 *  healthy).
 *
 *  Scans every agent in the map rather than two named ones — same answer for
 *  the pairs it used to handle, and it keeps answering once a session can hold
 *  more than two participants. */
export function worstHealth(h: SessionHealth | undefined): AgentHealth | undefined {
  if (!h) return undefined;
  const present = Object.values(h);
  return HEALTH_RANK.find((rank) => present.includes(rank));
}

/** App-wide footer summary: the worst state across all sessions + how many
 *  sessions sit in it. `ok` when nothing is retrying or dead; `idle` when no
 *  agents are tracked at all (health is event-driven + in-memory, so it's empty
 *  after a fresh launch until a reopened session's agent emits — don't claim
 *  "OK" then). */
export function appHealthSummary(bySession: Record<string, SessionHealth>): {
  state: "ok" | "retrying" | "stalled" | "dead" | "idle";
  count: number;
} {
  let dead = 0;
  let retrying = 0;
  let stalled = 0;
  for (const h of Object.values(bySession)) {
    const w = worstHealth(h);
    if (w === "dead") dead += 1;
    else if (w === "stalled") stalled += 1;
    else if (w === "retrying") retrying += 1;
  }
  if (dead > 0) return { state: "dead", count: dead };
  if (stalled > 0) return { state: "stalled", count: stalled };
  if (retrying > 0) return { state: "retrying", count: retrying };
  if (Object.keys(bySession).length === 0) return { state: "idle", count: 0 };
  return { state: "ok", count: 0 };
}
