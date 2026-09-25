import { isApproval, isTrayItem } from "../components/HaltBanner";
import { parseUtcMs } from "./time";

// Single source of the idle-unflagged attention badge — the label AND the
// tooltip. Both surfaces that render it derive from here: SessionTile (the
// dashboard card) and SessionView (the session header), which had the string
// duplicated verbatim, so a fix to one silently left the other stale. Same
// pattern as `phase.ts`: two widgets, one declaration, agreement by structure
// rather than by a hand-synced comment.

/**
 * The attention value the backend sets when a session goes idle with nothing
 * parked. `null`/absent = clear. Mirrors the Rust `session_attention` map, whose
 * only value today is this one.
 */
export const ATTENTION_IDLE_UNFLAGGED = "idle_unflagged";

/** Badge text. Short enough for the dashboard card's single line. */
export const ATTENTION_IDLE_LABEL = "NEEDS DIRECTION";

/**
 * Badge tooltip.
 *
 * Says "the session", not "the duo": a session runs N participants (dialog
 * default 1, cap 4), so naming a pair described a session shape most rosters do
 * not have. Both copies of this string said "the duo" until round 2 — they were
 * rendered text rather than doc prose, which is why round 1's framing sweep over
 * GENERAL_RULES, the public site, README and ARCHITECTURE went straight past
 * them.
 */
export const ATTENTION_IDLE_TOOLTIP =
  "Idle with no question or halt parked — the session was nudged to declare state";

// ---------------------------------------------------------------------------
// What a session is waiting on the USER for — the dashboard card and the
// header bell (the user, 2026-09-25: "gates also don't show on notification
// bell", and a halted session showed on neither).
// ---------------------------------------------------------------------------

/**
 * A session's needs, kept apart BY KIND. rc3 D35 dropped halts and gates from
 * these surfaces because one blended count read "needs your input" over an
 * empty tray; naming each kind is what lets them come back without that lie.
 */
export interface NeedsYou {
  /** Tray questions (`isTrayItem`). */
  questions: number;
  /** Approval gates (`isApproval`) — a gate holds the session until answered. */
  approvals: number;
  /** Halted and waiting for the user: an ordinary halt, or a temporary one
   *  whose wake time has PASSED — the wake is held while the session is
   *  paused, stopped or behind a pending gate, and then it is the user's move
   *  (EYES Q1). */
  halt: { reason: string; declaredBy: string } | null;
  /** A temporary halt still ahead of its wake time: waiting on something
   *  external, not on the user. */
  wakesAt: string | null;
}

/**
 * Every open session's needs, from the durable tray and the halt list. `now`
 * (epoch ms) decides whether a temporary halt is still waiting to wake —
 * nothing fires when a wake time passes without a wake, so callers re-render
 * on a clock. A legacy `halt` tray row counts as neither kind.
 */
export function needsYouBySession(
  pending: readonly { session_id: string; kind: string; options: readonly string[] }[],
  halts: readonly {
    session_id: string;
    declared_by: string;
    reason: string;
    wake_at: string | null;
  }[],
  now: number,
): Record<string, NeedsYou> {
  const out: Record<string, NeedsYou> = {};
  const entry = (sid: string) =>
    (out[sid] ??= { questions: 0, approvals: 0, halt: null, wakesAt: null });
  for (const p of pending) {
    if (isApproval(p)) entry(p.session_id).approvals += 1;
    else if (isTrayItem(p)) entry(p.session_id).questions += 1;
  }
  for (const h of halts) {
    const wake = h.wake_at ? parseUtcMs(h.wake_at) : Number.NaN;
    if (!Number.isNaN(wake) && wake > now) entry(h.session_id).wakesAt = h.wake_at;
    else entry(h.session_id).halt = { reason: h.reason, declaredBy: h.declared_by };
  }
  return out;
}

/** Does this session need the user at all? */
export function needsYou(n: NeedsYou | undefined): boolean {
  return !!n && (n.approvals > 0 || n.questions > 0 || n.halt !== null);
}

/** The needs, one phrase per kind — "approval waiting", "2 questions",
 *  "halted — your move" — never folded into one number. */
export function needsYouParts(n: NeedsYou): string[] {
  const parts: string[] = [];
  if (n.approvals > 0) {
    parts.push(n.approvals === 1 ? "approval waiting" : `${n.approvals} approvals waiting`);
  }
  if (n.questions > 0) parts.push(n.questions === 1 ? "1 question" : `${n.questions} questions`);
  if (n.halt) parts.push("halted — your move");
  return parts;
}

/** `wakes 14:05` — a temporary halt's wake time, in the user's local clock. */
export function wakesLabel(wakeAt: string): string {
  const t = new Date(parseUtcMs(wakeAt));
  const hh = String(t.getHours()).padStart(2, "0");
  const mm = String(t.getMinutes()).padStart(2, "0");
  return `wakes ${hh}:${mm}`;
}
