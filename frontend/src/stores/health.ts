import { create } from "zustand";

/** Agent liveness as reported by the backend `session:agent_health` event
 *  (mirrors Rust `AgentHealth::as_str`). */
export type AgentHealth = "running" | "retrying" | "dead";

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
  if (h.brian === "retrying" || h.rain === "retrying") return "retrying";
  if (h.brian === "running" || h.rain === "running") return "running";
  return undefined;
}
