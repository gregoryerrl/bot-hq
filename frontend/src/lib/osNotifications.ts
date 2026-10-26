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

// ---- urgent gates (feedback #52, the user's desktop tier) ----------------
//
// An URGENT gate is the one escalation that ignores the focus check, the
// cooldown and the coalescing: the parking agent said the user must see it
// NOW (a production incident), so it fires at once — focused or not — and
// again every ten minutes until the gate is answered, at most six repeats.
// No phone push: the user's pick was the desktop tier. The hook owns the
// timers; this is the policy.

/** Re-notify an unanswered urgent gate this often… */
export const URGENT_RENOTIFY_MS = 10 * 60_000;
/** …and at most this many times after the first notification. */
export const URGENT_RENOTIFY_MAX = 6;

export interface UrgentGate {
  choiceId: string;
  sessionId: string;
  agent: string;
  reason: string;
  command: string;
  /** Feedback #127: a forced approval notifies while it is still queued for
   *  the reviewer's read; the card is not the user's to answer yet, and the
   *  toast must not say "approval" as if it were. The promotion fires again
   *  without this flag. */
  underReview?: boolean;
}

/** The toast for an urgent gate: the reason first, then the command's first
 *  line — what the user needs to decide whether to open bot-hq now. `repeat`
 *  is 0 for the first notification, then 1..URGENT_RENOTIFY_MAX. */
export function urgentToast(g: UrgentGate, repeat: number): Toast {
  const command = g.command.split("\n").find((l) => l.trim() !== "")?.trim() ?? "";
  const still = repeat > 0 ? ` (still waiting, ${repeat}/${URGENT_RENOTIFY_MAX})` : "";
  const what = g.underReview ? "URGENT, under review" : "URGENT approval";
  const lead = g.underReview ? "reviewer reads it first — " : "";
  return {
    title: `bot-hq — ${what}${still}`,
    body: snip(`${lead}${command ? `${g.reason} — ${command}` : g.reason}`),
  };
}

/** Delay until the next re-notification after `repeats` already sent, or
 *  `null` once the cap is reached — the gate still shows red in the app. */
export function nextUrgentDelay(repeats: number): number | null {
  return repeats < URGENT_RENOTIFY_MAX ? URGENT_RENOTIFY_MS : null;
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
