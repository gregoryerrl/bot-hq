// Detection for scroll containers that can scroll HORIZONTALLY. Pure — no
// `node:` imports — so it stays safe to sit in `src/lib/`; the file walking lives
// in `overflow.test.ts`, which is the only caller. Same shape, and for the same
// reason, as `framing.ts`.
//
// House rule, user-mandated and absolute: **no horizontal scrolling, ever.**
//
// The mechanism is a CSS rule rather than a typo. When one axis of `overflow` is
// set to anything other than `visible`, the computed value of the OTHER axis
// becomes `auto`. So a bare `overflow-y-auto` IS a horizontal scroller.
// `html`/`body`/`#root` carry `overflow-x: hidden` as a backstop, but that is the
// page — an inner container scrolls on its own.

import { stripComments } from "./framing";

/**
 * Every class that makes an axis non-`visible`, and therefore makes the other
 * axis scroll unless it is pinned.
 *
 * `overflow-auto`, `overflow-x-auto` and `overflow-x-scroll` had ZERO live uses
 * when this shipped. They are listed anyway: covering them costs nothing today
 * and closes the obvious door to reintroducing the bug.
 */
export const SCROLL_AXIS_CLASSES = [
  "overflow-y-auto",
  "overflow-y-scroll",
  "overflow-auto",
  "overflow-scroll",
  "overflow-x-auto",
  "overflow-x-scroll",
  // Arbitrary-value forms of the same axis property — a `[overflow-y:auto]`
  // or `overflow-y-[auto]` is the same container in a different spelling
  // (round 7; both had zero uses when added, listed for the same reason as the
  // three above).
  "[overflow-y:auto]",
  "[overflow-y:scroll]",
  "[overflow:auto]",
  "[overflow:scroll]",
  "overflow-y-[auto]",
  "overflow-y-[scroll]",
] as const;

/**
 * What this guard structurally cannot see (round 7's frontend sweep, recorded
 * so nobody reads "0 violations" as "0 possible"): `.css`/`index.html`/the
 * plugin iframe's own stylesheet (the glob is `src/**\/*.{ts,tsx}`); inline
 * `style={{ overflowY: "auto" }}`; a class string built by concatenation; a
 * line holding two elements where only one is clipped; a variant-prefixed
 * clip (`md:overflow-x-hidden`) satisfying `includes` without unconditional
 * clipping; native scrollers with no class (`<textarea>`, xterm's viewport).
 * Each of those was measured at 0 on 2026-08-17; none is guarded here.
 */

/** The class that must accompany any of the above. */
export const HORIZONTAL_CLIP = "overflow-x-hidden";

/** A container that can scroll sideways: its 1-based line and the text. */
interface BareScrollContainer {
  line: number;
  text: string;
}

/**
 * Every non-comment line of `source` that opens a scroll container without
 * pairing it.
 *
 * **Comments are excluded**, the same exemption `framing.ts` and
 * `retired_identifier_test.rs` make: prose ABOUT the rule is a record, not a
 * violation. Load-bearing here — the first run of this sweep reported 12 sites
 * and four were comments explaining why the pair is correct, including
 * `FileViewerDialog`'s note reading "The pair, not a bare `overflow-auto`". A
 * guard that reports the documentation of a fix as an instance of the bug trains
 * people to ignore it.
 *
 * Reusing `stripComments` rather than re-deriving it is deliberate: a hand-rolled
 * per-line version written for this file could not see CONTINUATION lines, and
 * missed exactly that — two middle lines of a `{@literal /*} … *␘/}` block, both
 * starting with a backtick. The CL records the identical defect in
 * `cl_stale_refs`, whose retirement detection is per LINE, so a banner's names
 * land on continuations carrying no marker. One shared stripper, block-aware,
 * line-preserving.
 *
 * **Line-scoped, not file-scoped.** In every compliant site here the pair is
 * written on one line, so a line is the honest unit — and a file-scoped check
 * would let one compliant container vouch for a bare sibling. Not theoretical:
 * `ApprovalGate` was the ONE component whose pair was already asserted, by a test
 * that reads a single box by role, and it still carried a bare container that
 * test never saw.
 */
export function findBareScrollContainers(source: string): BareScrollContainer[] {
  const hits: BareScrollContainer[] = [];
  stripComments(source)
    .split("\n")
    .forEach((text, i) => {
      if (!SCROLL_AXIS_CLASSES.some((c) => text.includes(c))) return;
      if (text.includes(HORIZONTAL_CLIP)) return;
      hits.push({ line: i + 1, text: text.trim() });
    });
  return hits;
}
