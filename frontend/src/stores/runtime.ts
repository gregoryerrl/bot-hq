import type { AgentBusy, SessionActivity } from "./activity";
import type { AgentHealth } from "./health";
import { slotKey } from "../lib/participants";

/** One live session's runtime snapshot from the backend `get_session_runtime`
 *  command (Bug C backfill). Mirrors the Rust `SessionRuntime` (snake_case).
 *
 *  The `brian_*` / `rain_*` field names are frozen wire and name **turn slots 0
 *  and 1**, not agents — `src/tauri_cmd/sessions.rs` fills them from
 *  `handle.participants.get(0)` / `.get(1)`. See {@link slotKey}. */
export interface SessionRuntime {
  session_id: string;
  activity: string;
  brian_busy: boolean;
  rain_busy: boolean;
  brian_health: string | null;
  rain_health: string | null;
  router_alive: boolean | null;
  attention: string | null;
  working: string | null;
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
  setActivity: (id: string, a: SessionActivity, busy?: AgentBusy) => void,
  setHealth: (id: string, agent: string, h: AgentHealth) => void,
  setRouterHealth: (id: string, alive: boolean) => void,
  setAttention?: (id: string, state: string | null) => void,
  setWorking?: (id: string, reason: string | null) => void,
): void {
  for (const r of rows) {
    // Slot-shaped wire -> slot keys. Unpacking these under the literal slugs
    // `"brian"` / `"rain"` is what blanked every health dot after a restart:
    // no rc3 roster has those slugs, so `SessionView`'s per-participant lookup
    // missed every entry this seeded (rc3 D10).
    setActivity(r.session_id, r.activity as SessionActivity, {
      [slotKey(0)]: r.brian_busy,
      [slotKey(1)]: r.rain_busy,
    });
    if (r.brian_health)
      setHealth(r.session_id, slotKey(0), r.brian_health as AgentHealth);
    if (r.rain_health)
      setHealth(r.session_id, slotKey(1), r.rain_health as AgentHealth);
    // Null = solo / never reported → leave it (a missing entry reads as alive).
    if (r.router_alive !== null && r.router_alive !== undefined)
      setRouterHealth(r.session_id, r.router_alive);
    // Null = clear — pass it through so a stale pre-reload chip resets.
    if (setAttention) setAttention(r.session_id, r.attention ?? null);
    if (setWorking) setWorking(r.session_id, r.working ?? null);
  }
}
