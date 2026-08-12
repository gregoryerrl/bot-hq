import { create } from "zustand";

/** How full an agent's context window is, as of its LAST COMPLETED TURN.
 *
 *  Raw operands rather than a percentage — the tooltip shows "620K / 1M"
 *  alongside "62%", and the division is trivial to redo while the operands are
 *  not recoverable from a rounded float.
 *
 *  Backed by the `session:agent_context` event, which the backend emits only
 *  when claude-code reported a `contextWindow`. A gateway model whose provider
 *  omits it produces no event at all — so "no entry" means *unknown*, never
 *  *empty*, and the UI must render a gap rather than 0%. */
export type AgentContext = { usedTokens: number; contextWindow: number };

/** Participant slug → context occupancy. The slug is an internal key and is
 *  never rendered (rc3 D10) — see `lib/participants.ts` for what is. */
type SessionContext = Record<string, AgentContext | undefined>;

interface ContextStore {
  /** session_id -> per-agent context occupancy. Populated live from the
   *  `session:agent_context` event; in-memory only, resets on app restart.
   *  An agent with no entry has either not completed a turn yet or runs on a
   *  provider that doesn't report a window. */
  bySession: Record<string, SessionContext>;
  setContext: (sessionId: string, agent: string, ctx: AgentContext) => void;
  clearSession: (sessionId: string) => void;
}

export const useContextStore = create<ContextStore>((set) => ({
  bySession: {},
  setContext: (sessionId, agent, ctx) =>
    set((s) => ({
      bySession: {
        ...s.bySession,
        [sessionId]: { ...s.bySession[sessionId], [agent]: ctx },
      },
    })),
  clearSession: (sessionId) =>
    set((s) => {
      if (!s.bySession[sessionId]) return s;
      const bySession = { ...s.bySession };
      delete bySession[sessionId];
      return { bySession };
    }),
}));

/** Occupancy as a 0..1 fraction, or `undefined` when unknown.
 *
 *  Guards a zero window defensively: the backend already refuses to emit one,
 *  but a NaN leaking into a width style silently breaks the bar rather than
 *  failing loudly. */
export function contextFraction(c: AgentContext | undefined): number | undefined {
  if (!c || c.contextWindow <= 0) return undefined;
  return c.usedTokens / c.contextWindow;
}

/** Severity band for colouring. Thresholds are about *user action*, not
 *  aesthetics: below 70% there is nothing to do, 70–90% is "start thinking
 *  about wrapping up", above 90% is "wrap up now or risk a mid-task
 *  compaction". */
export function contextSeverity(fraction: number): "ok" | "warn" | "critical" {
  if (fraction >= 0.9) return "critical";
  if (fraction >= 0.7) return "warn";
  return "ok";
}

/** Compact token count for the tooltip: 619856 -> "620K", 1000000 -> "1.0M". */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}
