/**
 * OS-notification escalation — the pure half.
 *
 * The in-app bell counts parked questions (rc3 D35); OS escalation
 * deliberately covers MORE: every tray park (questions, approvals, gated
 * commands — all arrive as `session:pending_choice`) plus session halts
 * (`session:awaiting_user`). The hook queues events for a short burst window
 * and this module decides what actually fires, so the decision is testable
 * without a webview:
 *
 * - dedupe: one toast per (session, kind) per flush;
 * - cooldown: a (session, kind) that fired recently stays quiet — a
 *   park→supersede→re-park volley is one toast, not three;
 * - coalesce: three or more due events collapse into a single aggregate
 *   ("N sessions need you"), mirroring the Dashboard's sessions-awaiting
 *   aggregation — N halts landing together must not be N simultaneous toasts.
 */

export interface EscalationEvent {
  sessionId: string;
  kind: "question" | "halt";
  snippet: string;
}

export interface Toast {
  title: string;
  body: string;
}

/** A (session, kind) that fired within this window stays silent. */
export const COOLDOWN_MS = 60_000;
/** Queued events flush together after this long — the coalescing window. */
export const BURST_WINDOW_MS = 1_500;
/** At this many due events, one aggregate toast replaces the pile. */
export const BURST_THRESHOLD = 3;

const MAX_BODY = 140;

function snip(s: string): string {
  const t = s.trim().replace(/\s+/g, " ");
  return t.length > MAX_BODY ? `${t.slice(0, MAX_BODY - 1)}…` : t;
}

/**
 * Decide the toasts for one flush of the queue. Pure: callers pass the
 * last-fired map in and store the returned one, so repeat-parks and
 * simultaneous halts are policy here rather than accidents at the call site.
 */
export function planFlush(
  queued: EscalationEvent[],
  lastFired: Record<string, number>,
  now: number,
): { toasts: Toast[]; next: Record<string, number> } {
  // Carry only entries still inside their cooldown — the map self-limits to
  // the active window instead of growing per (session, kind) forever.
  const next: Record<string, number> = {};
  for (const [k, ts] of Object.entries(lastFired)) {
    if (now - ts < COOLDOWN_MS) next[k] = ts;
  }
  const due: EscalationEvent[] = [];
  const seen = new Set<string>();
  for (const ev of queued) {
    const key = `${ev.sessionId}:${ev.kind}`;
    if (seen.has(key)) continue;
    const last = next[key];
    if (last !== undefined && now - last < COOLDOWN_MS) continue;
    seen.add(key);
    next[key] = now;
    due.push(ev);
  }
  if (due.length === 0) return { toasts: [], next };
  if (due.length >= BURST_THRESHOLD) {
    const n = new Set(due.map((e) => e.sessionId)).size;
    return {
      toasts: [
        {
          title: "bot-hq",
          body: n === 1 ? "1 session needs you" : `${n} sessions need you`,
        },
      ],
      next,
    };
  }
  return {
    toasts: due.map((ev) => ({
      title: ev.kind === "halt" ? "bot-hq — session waiting" : "bot-hq — question parked",
      body: snip(ev.snippet),
    })),
    next,
  };
}

// ---- user preference (per-machine, mirrors the update-banner dismiss) ----

const PREF_KEY = "bot-hq:os-notifications";

/** Default ON; a denied OS permission is handled at send time, not here. */
export function osNotificationsEnabled(): boolean {
  try {
    return localStorage.getItem(PREF_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setOsNotificationsEnabled(on: boolean): void {
  try {
    localStorage.setItem(PREF_KEY, on ? "on" : "off");
  } catch {
    /* storage unavailable — the toggle just won't persist */
  }
}
