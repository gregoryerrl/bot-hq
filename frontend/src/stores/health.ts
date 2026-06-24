import { create } from "zustand";

/** Agent liveness as reported by the backend `session:agent_health` event
 *  (mirrors Rust `AgentHealth::as_str`). */
export type AgentHealth = "running" | "retrying" | "stalled" | "dead";

type SessionHealth = { brian?: AgentHealth; rain?: AgentHealth };

interface HealthStore {
  /** session_id -> per-agent health. Populated live from the agent_health
   *  event; an agent with no entry is assumed running (events fire only on
   *  transitions). In-memory only — resets on app restart. */
  bySession: Record<string, SessionHealth>;
  setHealth: (sessionId: string, agent: string, health: AgentHealth) => void;
  clearSession: (sessionId: string) => void;
}

export const useHealthStore = create<HealthStore>((set) => ({
  bySession: {},
  setHealth: (sessionId, agent, health) =>
    set((s) => ({
      bySession: {
        ...s.bySession,
        [sessionId]: { ...s.bySession[sessionId], [agent]: health },
      },
    })),
  clearSession: (sessionId) =>
    set((s) => {
      if (!s.bySession[sessionId]) return s;
      const next = { ...s.bySession };
      delete next[sessionId];
      return { bySession: next };
    }),
}));

/** Worst-of a session's agents, for a single tile-level dot: dead > retrying >
 *  running. Returns undefined when there's no health data (assume healthy). */
export function worstHealth(h: SessionHealth | undefined): AgentHealth | undefined {
  if (!h) return undefined;
  if (h.brian === "dead" || h.rain === "dead") return "dead";
  if (h.brian === "stalled" || h.rain === "stalled") return "stalled";
  if (h.brian === "retrying" || h.rain === "retrying") return "retrying";
  if (h.brian === "running" || h.rain === "running") return "running";
  return undefined;
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
