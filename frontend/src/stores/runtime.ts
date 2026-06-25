import type { SessionActivity } from "./activity";
import type { AgentHealth } from "./health";

/** One live session's runtime snapshot from the backend `get_session_runtime`
 *  command (Bug C backfill). Mirrors the Rust `SessionRuntime` (snake_case). */
export interface SessionRuntime {
  session_id: string;
  activity: string;
  brian_health: string | null;
  rain_health: string | null;
  router_alive: boolean | null;
}

/** Seed the event-driven activity + health stores from a one-shot runtime
 *  snapshot. Those stores are otherwise populated only by `session:activity` /
 *  `session:agent_health` events, which fire on transitions and can be missed
 *  during the respawn window before the React listeners mount — leaving the
 *  footer / tiles / input-indicator stale until the next transition. Pure: takes
 *  the store setters so it's unit-testable. Null health = agent not yet tracked
 *  → skip (a missing entry reads as healthy, same as a missing event). */
export function seedRuntimeStores(
  rows: SessionRuntime[],
  setActivity: (id: string, a: SessionActivity) => void,
  setHealth: (id: string, agent: string, h: AgentHealth) => void,
  setRouterHealth: (id: string, alive: boolean) => void,
): void {
  for (const r of rows) {
    setActivity(r.session_id, r.activity as SessionActivity);
    if (r.brian_health)
      setHealth(r.session_id, "brian", r.brian_health as AgentHealth);
    if (r.rain_health)
      setHealth(r.session_id, "rain", r.rain_health as AgentHealth);
    // Null = solo / never reported → leave it (a missing entry reads as alive).
    if (r.router_alive !== null && r.router_alive !== undefined)
      setRouterHealth(r.session_id, r.router_alive);
  }
}
