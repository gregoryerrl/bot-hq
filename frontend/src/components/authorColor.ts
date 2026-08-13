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
 * The tokens are Tailwind palette names (hexes in `tailwind.config.ts`), not
 * agent identities — nothing renders them.
 *
 * **There used to be two**, with a comment calling a repeat "a shared colour,
 * not a wrong one" — defensible while a session held two participants, false
 * the moment it could hold four. It shipped as two against a roster of three, so
 * a collision was not unlucky but CERTAIN, and the user reported it from a live
 * session within the hour: *"HANDS and EYES-2 have the same color."*
 */
/**
 * One entry of the participant palette (rc3 **D20**).
 *
 * `name` is what a user picks from a list; `token` is the Tailwind class the
 * byline renders. Exported as DATA so the picker and the rotation read the same
 * eight entries — a picker built from its own hardcoded list is how a user
 * chooses a colour nothing renders.
 */
export type ParticipantColor = { name: string; token: string };

/**
 * The palette, in rotation order.
 *
 * **Named by colour, never by agent.** These were `author-brian` and
 * `author-rain` — two agent names living on in the design system after rc3 D10
 * retired them everywhere else.
 *
 * Eight, against a roster cap of four: the rotation only needs four to
 * guarantee distinctness, and a PICKER wants more than the cap, or a user who
 * dislikes a hue has no alternative left that is still distinct from everyone
 * else's.
 */
export const PARTICIPANT_COLORS: readonly ParticipantColor[] = [
  { name: "Orange", token: "text-author-orange" },
  { name: "Violet", token: "text-author-violet" },
  { name: "Cyan", token: "text-author-cyan" },
  { name: "Rose", token: "text-author-rose" },
  { name: "Lime", token: "text-author-lime" },
  { name: "Amber", token: "text-author-amber" },
  { name: "Sky", token: "text-author-sky" },
  { name: "Pink", token: "text-author-pink" },
] as const;

const PARTICIPANT_HUES = PARTICIPANT_COLORS.map((c) => c.token);

/** The palette entry a stored colour NAME refers to, or `undefined`. Case- and
 *  whitespace-insensitive, because the name round-trips through storage. */
export function colorByName(name: string | null | undefined): ParticipantColor | undefined {
  if (!name) return undefined;
  const want = name.trim().toLowerCase();
  return PARTICIPANT_COLORS.find((c) => c.name.toLowerCase() === want);
}

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

/**
 * The hue for turn slot `i` — the ROTATION rc3 D20 asks for.
 *
 * Wraps past the palette, which only matters above the roster cap of four: at
 * or below it, two participants cannot share a hue at all.
 */
export function participantHue(slot: number): string {
  return PARTICIPANT_HUES[slot % PARTICIPANT_HUES.length];
}

/**
 * `label` → hue class.
 *
 * `hues` is the roster's slot assignment ([`participantHueIndex`]) and is what
 * makes two participants in one session distinguishable BY CONSTRUCTION. Pass it
 * wherever the roster is in hand.
 *
 * Without it the hash is the fallback: stable, and distinct only by luck. That
 * is fine for a surface showing one author at a time (a dashboard tile), and not
 * fine for a chat stream where the whole job of the colour is telling two
 * speakers apart.
 */
export function authorColorClass(
  label: string,
  hues?: Record<string, string>,
) {
  if (!label) return NEUTRAL;
  const reserved = RESERVED.get(label);
  if (reserved) return reserved;
  return hues?.[label] ?? PARTICIPANT_HUES[hash(label) % PARTICIPANT_HUES.length];
}
