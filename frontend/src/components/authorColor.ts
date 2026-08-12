/**
 * Display LABEL → colour class.
 *
 * Keyed on what is rendered, not on an internal key. One participant reaches
 * the chat byline through `messages.author` (a slug) and the turn-status line
 * through a slot key (`lib/participants.ts`), and its resolved `ROLE · Model`
 * label is the only string those two surfaces have in common — so hashing the
 * label is what keeps one participant one colour everywhere.
 *
 * It also un-breaks the tinting. The map used to hold two agent slugs; once
 * slugs became role-derived (rc3 D10) every participant missed it and fell
 * through to the neutral tone, so bylines lost their per-participant colour
 * altogether.
 */

import { UNKNOWN_PARTICIPANT } from "../lib/participants";

const NEUTRAL = "text-on-surface-variant";

/**
 * Labels that are NOT one participant, by the string `authorLabel` resolves
 * them to.
 *
 * `UNKNOWN_PARTICIPANT` belongs here rather than in the hue rotation: it is
 * what an author the roster cannot place reads as, so every unplaceable author
 * shares it. Hashing it would hand one hue to a crowd and imply they are the
 * same participant.
 *
 * A `Map`, not an object literal — an object would answer `"constructor"` and
 * `"toString"` out of `Object.prototype` and hand a FUNCTION back as a class
 * name for a participant unlucky enough to be labelled that.
 */
const RESERVED = new Map<string, string>([
  ["You", "text-author-user"],
  ["System", NEUTRAL],
  [UNKNOWN_PARTICIPANT, NEUTRAL],
]);

/**
 * Participant hues, assigned by hash.
 *
 * `author-brian` / `author-rain` are Tailwind palette TOKEN names (two hexes in
 * `tailwind.config.ts`), not agent identities — nothing renders them. The
 * palette carries exactly two participant hues, so a roster of three or more
 * repeats one; that is a shared colour, not a wrong one.
 */
const PARTICIPANT_HUES = ["text-author-brian", "text-author-rain"] as const;

/** FNV-1a, 32-bit. Any stable string→int would do; this one is short and has
 *  no dependencies. Stability is the whole requirement: the same label must
 *  pick the same hue on every surface and across restarts. */
function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

export function authorColorClass(label: string) {
  if (!label) return NEUTRAL;
  return (
    RESERVED.get(label) ?? PARTICIPANT_HUES[hash(label) % PARTICIPANT_HUES.length]
  );
}
