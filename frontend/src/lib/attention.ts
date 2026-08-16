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
